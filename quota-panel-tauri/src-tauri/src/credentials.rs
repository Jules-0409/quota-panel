use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use base64::prelude::*;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryAuth {
    pub access_token: String,
    pub active_organization_id: Option<String>,
}

fn get_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// 把 SQLite 的 value 列转成字符串。
///
/// VS Code 系的 `state.vscdb` 里 value 通常是 TEXT，但也可能是 BLOB 或数字，
/// 所以统一按 `rusqlite::types::Value` 取出再转换，
/// 避免单个键的类型不匹配让整条查询（乃至整个数据源）废掉。
fn sqlite_value_to_string(value: rusqlite::types::Value) -> Option<String> {
    use rusqlite::types::Value as SqlValue;
    match value {
        SqlValue::Null => None,
        SqlValue::Integer(i) => Some(i.to_string()),
        SqlValue::Real(f) => Some(f.to_string()),
        SqlValue::Text(s) => Some(s),
        SqlValue::Blob(b) => String::from_utf8(b).ok(),
    }
}

/// 打开 `state.vscdb`，优先**只读**。
///
/// 只读是首选：这些库是 Cursor / Devin Desktop 正在使用的活文件，只读连接不会抢写锁。
/// 实测这台机器上 Cursor 的 WAL 库（包括 `-wal` 非空、`-shm` 缺失的崩溃现场）
/// 纯 `SQLITE_OPEN_READ_ONLY` 也能打开，所以正常情况下走的都是只读路径。
///
/// 但只读并非在所有环境下都成立：只读连接无法创建 `-shm`。以下情况只读打开会失败：
/// 需要 WAL 恢复而库所在目录 / 文件系统不可写、Windows 上文件被独占或权限受限，
/// 以及**Cursor 应用自己正以读写方式持有这个库**——它是常驻的写入方，我们的连接只是
/// 众多读者之一。
/// 因此保留一条降级路径：
/// - 不带 `SQLITE_OPEN_CREATE`：库文件不存在时照样报错，绝不新建文件；
/// - 降级后立即 `PRAGMA query_only`，这条连接发不出任何写语句
///   （实测写入返回 "attempt to write a readonly database"），
///   只会按需重建 `-wal` / `-shm`，与 Cursor / VS Code 自己打开这个库的行为一致。
fn open_vscdb_readonly(db_path: &Path) -> Result<Connection, String> {
    let ro_flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = match Connection::open_with_flags(db_path, ro_flags) {
        Ok(conn) => conn,
        Err(ro_err) => {
            let rw_flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
            let conn = Connection::open_with_flags(db_path, rw_flags).map_err(|rw_err| {
                format!(
                    "无法打开 SQLite 数据库 {}（只读失败：{}；降级读写也失败：{}）",
                    db_path.display(),
                    ro_err,
                    rw_err
                )
            })?;
            conn.pragma_update(None, "query_only", true).map_err(|e| {
                format!("无法把 {} 设为只读查询模式：{}", db_path.display(), e)
            })?;
            conn
        }
    };

    // 活文件可能被对方短暂独占，给一点点等待时间，而不是一撞锁就失败
    let _ = conn.busy_timeout(std::time::Duration::from_millis(500));

    Ok(conn)
}

/// 从 VS Code 系的 `state.vscdb`（SQLite，表结构 `ItemTable(key, value)`）里读取指定键。
///
/// 用 bundled rusqlite 直接读，不再 spawn `sqlite3` / `python3` / PowerShell：
/// Windows 10/11 不自带 sqlite3.exe，普通 Windows 也没装 System.Data.SQLite 程序集，
/// 那些外部命令会「命令找不到 → if let Ok(...) 静默失败」，是排查困难的根源。
///
/// 返回的 map 只包含实际存在的键；打不开或查询失败时返回带路径的可读错误，不静默吞掉。
pub(crate) fn read_vscdb_items(
    db_path: &Path,
    keys: &[&str],
) -> Result<HashMap<String, String>, String> {
    let conn = open_vscdb_readonly(db_path)?;

    let mut stmt = conn
        .prepare("SELECT value FROM ItemTable WHERE key = ?1")
        .map_err(|e| format!("查询 {} 的 ItemTable 失败：{}", db_path.display(), e))?;

    let mut out = HashMap::new();
    for key in keys {
        let row = stmt.query_row(rusqlite::params![key], |row| {
            row.get::<_, rusqlite::types::Value>(0)
        });
        match row {
            Ok(value) => {
                if let Some(s) = sqlite_value_to_string(value) {
                    out.insert((*key).to_string(), s);
                }
            }
            // 键不存在属于正常情况（没登录 / 版本不同），跳过即可
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(e) => {
                return Err(format!(
                    "从 {} 读取键 {} 失败：{}",
                    db_path.display(),
                    key,
                    e
                ));
            }
        }
    }
    Ok(out)
}

/// Read password from macOS Keychain using `security` CLI
#[cfg(target_os = "macos")]
fn read_macos_keychain(service: &str, account: &str) -> Option<String> {
    let output = Command::new("security")
        .args(["find-generic-password", "-s", service, "-a", account, "-w"])
        .output()
        .ok()?;

    if output.status.success() {
        let val = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !val.is_empty() {
            return Some(val);
        }
    }
    None
}

/// Read credential from Windows Credential Manager using PowerShell
#[cfg(target_os = "windows")]
fn read_windows_credential(target: &str) -> Option<Vec<u8>> {
    let ps_cmd = format!(
        r#"$sig = @"
using System;
using System.Runtime.InteropServices;
public class QPCred {{
  [DllImport("advapi32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
  public static extern bool CredRead(string target, int type, int flags, out IntPtr credential);
  [DllImport("advapi32.dll")]
  public static extern void CredFree(IntPtr cred);
  [StructLayout(LayoutKind.Sequential)]
  public struct CREDENTIAL {{
    public int Flags; public int Type; public IntPtr TargetName; public IntPtr Comment;
    public long LastWritten; public int CredentialBlobSize; public IntPtr CredentialBlob;
    public int Persist; public int AttributeCount; public IntPtr Attributes;
    public IntPtr TargetAlias; public IntPtr UserName;
  }}
  public static byte[] Get(string target) {{
    IntPtr p;
    if (!CredRead(target, 1, 0, out p)) return null;
    CREDENTIAL cr = (CREDENTIAL)Marshal.PtrToStructure(p, typeof(CREDENTIAL));
    byte[] b = new byte[cr.CredentialBlobSize];
    Marshal.Copy(cr.CredentialBlob, b, 0, cr.CredentialBlobSize);
    CredFree(p);
    return b;
  }}
}}
"@
Add-Type -TypeDefinition $sig
$b = [QPCred]::Get("{}")
if ($b -ne $null) {{ [Convert]::ToBase64String($b) }}
"#,
        target
    );

    for exe in ["powershell", "pwsh"] {
        if let Ok(output) = Command::new(exe)
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
            .output()
        {
            if output.status.success() {
                let out_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if let Ok(decoded) = BASE64_STANDARD.decode(out_str.as_bytes()) {
                    return Some(decoded);
                }
            }
        }
    }
    None
}

/// Retrieve the Factory 32-byte AES key from Keychain or Windows Credential Manager
pub fn load_factory_key() -> Result<Vec<u8>, String> {
    let service = "Factory CLI";

    #[cfg(target_os = "macos")]
    {
        let accounts = ["auth-encryption-key-security-cli", "auth-encryption-key"];
        for account in accounts {
            if let Some(val) = read_macos_keychain(service, account) {
                if let Ok(key) = BASE64_STANDARD.decode(val.as_bytes()) {
                    if key.len() == 32 {
                        return Ok(key);
                    }
                }
            }
        }
        return Err("Cannot find 32-byte Factory key in macOS Keychain".into());
    }

    #[cfg(target_os = "windows")]
    {
        let targets = [
            "Factory CLI/auth-encryption-key",
            "Factory CLI/auth-encryption-key-security-cli",
        ];
        for target in targets {
            if let Some(blob) = read_windows_credential(target) {
                // Blob could be direct 32 bytes or base64 utf8 string
                if blob.len() == 32 {
                    return Ok(blob);
                }
                if let Ok(s) = String::from_utf8(blob) {
                    if let Ok(key) = BASE64_STANDARD.decode(s.trim().as_bytes()) {
                        if key.len() == 32 {
                            return Ok(key);
                        }
                    }
                }
            }
        }
        return Err("Cannot find 32-byte Factory key in Windows Credential Manager".into());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err("Unsupported operating system for keyring retrieval".into())
    }
}

/// Decrypt Factory credentials from ~/.factory/auth.v2.loginkeychain or ~/.factory/auth.v2.keyring
pub fn load_factory_credentials() -> Result<FactoryAuth, String> {
    let home = get_home_dir().ok_or("Cannot locate user home directory")?;
    let candidates = [
        home.join(".factory").join("auth.v2.loginkeychain"),
        home.join(".factory").join("auth.v2.keyring"),
    ];

    let path = candidates
        .iter()
        .find(|p| p.exists())
        .ok_or_else(|| "Factory auth file not found in ~/.factory/".to_string())?;

    let file_content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read Factory keyring file: {}", e))?;

    let parts: Vec<&str> = file_content.trim().split(':').collect();
    if parts.len() != 3 {
        return Err("Invalid Factory credential format; expected iv:authTag:ciphertext".into());
    }

    let iv = BASE64_STANDARD
        .decode(parts[0].as_bytes())
        .map_err(|e| format!("Invalid base64 iv: {}", e))?;
    let tag = BASE64_STANDARD
        .decode(parts[1].as_bytes())
        .map_err(|e| format!("Invalid base64 tag: {}", e))?;
    let ciphertext = BASE64_STANDARD
        .decode(parts[2].as_bytes())
        .map_err(|e| format!("Invalid base64 ciphertext: {}", e))?;

    let key_bytes = load_factory_key()?;
    if key_bytes.len() != 32 {
        return Err("Factory AES key must be 32 bytes".into());
    }

    let plaintext = decrypt_factory_payload(&key_bytes, &iv, &tag, &ciphertext)?;

    let auth: FactoryAuth = serde_json::from_slice(&plaintext)
        .map_err(|e| format!("Failed to parse decrypted Factory credentials JSON: {}", e))?;

    Ok(auth)
}

/// 校验 GCM 认证 tag，然后用 AES-256-CTR 解密 Droid CLI 写出的登录态（AAD 为空）。
///
/// 手搓 GCM 是有正当理由的：Droid CLI 是 Node.js 写的，IV 是 16 字节，
/// 而 `aes-gcm` crate 的 `Aes256Gcm` 把 nonce 类型固定成 `Nonce<U12>`（12 字节），处理不了。
/// 这里按 NIST SP 800-38D 从 16 字节 IV 推导 J0。
fn decrypt_factory_payload(
    key_bytes: &[u8],
    iv: &[u8],
    tag: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
    use aes::cipher::{BlockEncrypt, KeyInit};
    use aes::Aes256;
    use ghash::universal_hash::generic_array::GenericArray;
    use ctr::cipher::{KeyIvInit, StreamCipher};
    use ghash::universal_hash::UniversalHash;
    use ghash::GHash;

    type Aes256Ctr32BE = ctr::Ctr32BE<Aes256>;

    // 兼容 NIST SP 800-38D / Node.js OpenSSL:
    // 当 IV 为 16 字节时，通过 GHASH 计算初始计数器 J0，再用 CTR 解密密文

    // 0. H = AES_K(0^128)：GCM 的哈希子密钥，J0 推导和认证 tag 都要用
    let cipher_block = Aes256::new_from_slice(key_bytes)
        .map_err(|e| format!("Invalid AES key: {}", e))?;
    let mut h = [0u8; 16];
    cipher_block.encrypt_block((&mut h).into());

    if iv.len() != 12 && iv.len() < 16 {
        return Err(format!(
            "Invalid Factory credential format; unsupported iv length {} bytes (expected 12 or >= 16)",
            iv.len()
        ));
    }

    let j0 = if iv.len() == 12 {
        let mut j = [0u8; 16];
        j[0..12].copy_from_slice(&iv);
        j[15] = 1;
        j
    } else {
        // 1. 计算 J0 = GHASH_H(iv || 0^8 || [len(iv)]_64)
        let mut ghash = GHash::new(GenericArray::from_slice(&h));
        let b1 = GenericArray::clone_from_slice(&iv[0..16]);
        let mut b2_bytes = [0u8; 16];
        b2_bytes[15] = (iv.len() as u64 * 8) as u8;
        let b2 = GenericArray::clone_from_slice(&b2_bytes);
        ghash.update(&[b1, b2]);

        let mut out_j0 = [0u8; 16];
        let tag_val = ghash.finalize();
        out_j0.copy_from_slice(tag_val.as_slice());
        out_j0
    };

    // 2. 校验 GCM 认证 tag：tag = GHASH_H(A || C || [len(A)]_64 || [len(C)]_64) XOR E_K(J0)
    //    这里 AAD 为空，所以只有 C 参与。必须在 CTR 解密**之前**用原始密文计算，
    //    因为下面的 apply_keystream 是原地改写 ciphertext 的。
    if tag.len() != 16 {
        return Err(format!(
            "Invalid Factory credential format; authTag must be 16 bytes, got {}",
            tag.len()
        ));
    }

    let mut tag_ghash = GHash::new(GenericArray::from_slice(&h));
    tag_ghash.update_padded(ciphertext); // C，末尾不足一块自动补零
    let mut len_block = [0u8; 16];
    // 前 8 字节是 len(A) 的比特数（AAD 为空，恒为 0），后 8 字节是 len(C) 的比特数，
    // 都必须是完整的 64 位大端表示
    len_block[8..16].copy_from_slice(&((ciphertext.len() as u64) * 8).to_be_bytes());
    tag_ghash.update(&[GenericArray::clone_from_slice(&len_block)]);

    let mut ek_j0 = GenericArray::clone_from_slice(&j0);
    cipher_block.encrypt_block(&mut ek_j0);

    // T = GHASH_H(C || 0^u || [len(A)]_64 || [len(C)]_64) XOR E_K(J0)
    // 少了这一步 XOR 就是拿原始 GHASH 结果去比 tag，永远校验不过
    let mut computed_tag = tag_ghash.finalize();
    for i in 0..16 {
        computed_tag[i] ^= ek_j0[i];
    }

    // 常量时间比较：逐字节累积差异，避免 == 的提前退出泄露信息
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= computed_tag[i] ^ tag[i];
    }
    if diff != 0 {
        return Err("解密失败：加密密钥不匹配".into());
    }

    // 3. J1 = J0 + 1 (最后4字节大端自增) 作为 CTR 解密的初始块
    let mut j1 = j0;
    let ctr_num = u32::from_be_bytes(j0[12..16].try_into().unwrap());
    j1[12..16].copy_from_slice(&(ctr_num.wrapping_add(1)).to_be_bytes());

    // 4. 用 J1 作为初始计数器做 CTR 解密（原地改写密文的副本，原始密文已在上面喂给 GHASH）
    let mut plaintext = ciphertext.to_vec();
    let mut ctr_cipher = Aes256Ctr32BE::new((&key_bytes[..]).into(), (&j1).into());
    ctr_cipher.apply_keystream(&mut plaintext);

    Ok(plaintext)
}

/// Retrieve Devin token:
/// 1. From Devin CLI `credentials.toml`
/// 2. From Devin Desktop `state.vscdb` (read in-process with bundled rusqlite)
pub fn load_devin_token() -> Result<(String, String), String> {
    let home = get_home_dir().ok_or("Cannot locate user home directory")?;

    // Check Devin CLI credentials.toml candidates
    let cli_candidates = [
        home.join(".config").join("devin").join("credentials.toml"),
        home.join(".codeium").join("windsurf").join("credentials.toml"),
        home.join("AppData").join("Roaming").join("devin").join("credentials.toml"),
        home.join("Library").join("Application Support").join("devin").join("credentials.toml"),
    ];

    for path in &cli_candidates {
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("windsurf_api_key") || trimmed.starts_with("api_key") {
                        if let Some((_, v)) = trimmed.split_once('=') {
                            let clean = v.trim().trim_matches('"').trim_matches('\'');
                            if !clean.is_empty() {
                                return Ok((clean.to_string(), "Devin CLI 凭据".to_string()));
                            }
                        }
                    }
                }
            }
        }
    }

    // Check Devin Desktop state.vscdb candidates
    let db_candidates = [
        home.join("Library")
            .join("Application Support")
            .join("Devin")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb"),
        home.join("Library")
            .join("Application Support")
            .join("Windsurf")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb"),
        home.join("AppData")
            .join("Roaming")
            .join("Devin")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb"),
        home.join(".config")
            .join("Devin")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb"),
    ];

    let mut db_errors: Vec<String> = Vec::new();

    for db_path in &db_candidates {
        if !db_path.exists() {
            continue;
        }

        let items = match read_vscdb_items(db_path, &["windsurfAuthStatus"]) {
            Ok(items) => items,
            Err(e) => {
                db_errors.push(e);
                continue;
            }
        };

        let json_str = match items.get("windsurfAuthStatus") {
            Some(s) => s,
            // 这个库里没有 Devin 登录态，继续看下一个候选路径
            None => continue,
        };

        match serde_json::from_str::<serde_json::Value>(json_str) {
            Ok(v) => {
                if let Some(key) = v.get("apiKey").and_then(|k| k.as_str()) {
                    if !key.is_empty() {
                        return Ok((key.to_string(), "Devin Desktop 登录态".to_string()));
                    }
                }
            }
            Err(e) => db_errors.push(format!(
                "解析 {} 里的 windsurfAuthStatus 失败：{}",
                db_path.display(),
                e
            )),
        }
    }

    let mut err = "No valid Devin login session or credentials found".to_string();
    if !db_errors.is_empty() {
        err = format!("{}（读取 state.vscdb 时出错：{}）", err, db_errors.join("；"));
    }
    Err(err)
}
