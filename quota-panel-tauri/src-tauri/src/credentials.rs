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
                    "cred.sqlite_open: path={} ro={} rw={}",
                    db_path.display(),
                    ro_err,
                    rw_err
                )
            })?;
            conn.pragma_update(None, "query_only", true).map_err(|e| {
                format!("cred.sqlite_readonly_mode: path={} err={}", db_path.display(), e)
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
        .map_err(|e| format!("cred.sqlite_query: path={} err={}", db_path.display(), e))?;

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
                    "cred.key_read: path={} key={} err={}",
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
    #[cfg(target_os = "macos")]
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
        Err("Cannot find 32-byte Factory key in Windows Credential Manager".into())
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
        j[0..12].copy_from_slice(iv);
        j[15] = 1;
        j
    } else {
        // 1. 计算 J0 = GHASH_H(IV || 0^s || [len(IV)]_64)，s 把 IV 补零到 128 位边界
        //    （NIST SP 800-38D 5.2.1.2）。原来的写法有两个 bug：
        //      - 只喂了 IV 的前 16 字节，超过 16 字节的部分被丢掉；
        //      - 长度块写成 `b2[15] = (len*8) as u8`，IV ≥ 32 字节时 256 被截成 0。
        //    16 字节 IV 恰好两处都不受影响（128 落在最低字节、IV 也只有一块），
        //    所以实测一直是对的；但 32 字节的 IV 会解不开。
        let mut ghash = GHash::new(GenericArray::from_slice(&h));
        ghash.update_padded(iv); // IV 的全部字节，末尾按需补零到块边界

        let mut b2_bytes = [0u8; 16];
        // 前 8 字节是 AAD 长度（此处无 AAD，恒为 0），后 8 字节是 IV 的比特数
        b2_bytes[8..16].copy_from_slice(&((iv.len() as u64) * 8).to_be_bytes());
        ghash.update(&[GenericArray::clone_from_slice(&b2_bytes)]);

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
        return Err("cred.decrypt_key_mismatch".into());
    }

    // 3. J1 = J0 + 1 (最后4字节大端自增) 作为 CTR 解密的初始块
    let mut j1 = j0;
    let ctr_num = u32::from_be_bytes(j0[12..16].try_into().unwrap());
    j1[12..16].copy_from_slice(&(ctr_num.wrapping_add(1)).to_be_bytes());

    // 4. 用 J1 作为初始计数器做 CTR 解密（原地改写密文的副本，原始密文已在上面喂给 GHASH）
    let mut plaintext = ciphertext.to_vec();
    let mut ctr_cipher = Aes256Ctr32BE::new(key_bytes.into(), (&j1).into());
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
                                return Ok((clean.to_string(), "Devin CLI credentials".to_string()));
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
                        return Ok((key.to_string(), "Devin Desktop session".to_string()));
                    }
                }
            }
            Err(e) => db_errors.push(format!(
                "cred.devin_windsurf_parse: path={} err={}",
                db_path.display(),
                e
            )),
        }
    }

    let mut err = "devin.no_session".to_string();
    if !db_errors.is_empty() {
        err = format!("devin.no_session: state.vscdb errors: {}", db_errors.join("; "));
    }
    Err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用密钥与明文。密钥是 0x00..0x1f 的固定序列，明文形如真实的
    /// Droid 登录态 JSON，方便断言解密结果本身而不只是「没报错」。
    const PLAINTEXT: &str =
        r#"{"access_token":"test-token-abc123","active_organization_id":"org_test_42"}"#;

    /// 0x00..0x1f：与生成测试向量时 Node 侧用的 `000102...1f` 是同一把钥匙。
    fn key() -> Vec<u8> {
        (0..32u8).collect()
    }

    fn decode(s: &str) -> Vec<u8> {
        BASE64_STANDARD.decode(s.as_bytes()).unwrap()
    }

    /// 这些向量由 Node 的 `crypto.createCipheriv('aes-256-gcm', ...)` 生成，
    /// 生成脚本在仓库的 `scripts/gen-gcm-vectors.cjs`（改密钥/明文后可重跑）。
    ///
    /// **为什么必须用 Node 生成的向量**：Droid CLI 是 Node 写的，它的 IV 是 16 字节，
    /// 而 `aes-gcm` crate 只支持 12 字节 nonce——这正是这里手搓 GCM 的原因。
    /// 拿 Node 真实产出的密文来喂 `decrypt_factory_payload`，才能证明我们的
    /// J0 推导、tag 校验、CTR 计数器和写出方一致，而不是自己和自己对得上。
    #[test]
    fn decrypts_node_openssl_vector_with_16_byte_iv() {
        let iv = decode("AQIDBAUGBwgJCgsMDQ4PEA==");
        let tag = decode("2IRK7xud3vnzIqqV+C2psg==");
        let ct = decode(
            "nKf1tb4pHdnOo1AteNoeKgArGDg4EpahJYCnk0N2iocI1Mpd813L5bkKew5u19voZt8ICMLZdGHXOjpv7Bp+8TqcLcvhYWQ8+PyV",
        );

        let out = decrypt_factory_payload(&key(), &iv, &tag, &ct).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), PLAINTEXT);
    }

    /// 12 字节 IV 走 NIST 的 J0 = IV || 0^31 || 1 分支，是另一条代码路径。
    #[test]
    fn decrypts_node_openssl_vector_with_12_byte_iv() {
        let iv = decode("AQIDBAUGBwgJCgsM");
        let tag = decode("fstVQNFMf2Z6ADQSCckyRg==");
        let ct = decode("fsg7to/xg/UT1gwsdX3IEmA3hI3jQCSqyjqXCf/UFfS217nzTNSP69na37qqqZ888J81gaDwWR9pFGjvVSNbfhAif0bMhQLUeTDN");

        let out = decrypt_factory_payload(&key(), &iv, &tag, &ct).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), PLAINTEXT);
    }

    /// 32 字节 IV 也必须能解（走 GHASH 推导 J0 的分支，且补位长度是 256 而不是 128）。
    #[test]
    fn decrypts_node_openssl_vector_with_32_byte_iv() {
        let iv = decode("AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA=");
        let tag = decode("k3mXhRxDgbPlLMqaX0CT4g==");
        let ct = decode("qN9DfY/i3e7K9MTdoLb456jAowzBYuX30j0RIUgOGB/h9muo4uNJTpFodwiMJsddK+Mbogka6X1bJm+lvpTYER8BMEjUHbn3GMJ7");

        let out = decrypt_factory_payload(&key(), &iv, &tag, &ct).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), PLAINTEXT);
    }

    /// 空明文：密文长度为 0，tag 仍然必须校验通过（易漏的边界）。
    #[test]
    fn decrypts_empty_plaintext() {
        let iv = decode("AQIDBAUGBwgJCgsMDQ4PEA==");
        let tag = decode("lTLod13rv968aNf+3/LCJg==");

        let out = decrypt_factory_payload(&key(), &iv, &tag, &[]).unwrap();
        assert!(out.is_empty());
    }

    /// 认证失败必须报错，绝不能把垃圾解出来当凭据用。
    /// 这里改 tag 的第一个字节，模拟密钥不匹配 / 文件被篡改。
    #[test]
    fn wrong_tag_is_rejected() {
        let iv = decode("AQIDBAUGBwgJCgsMDQ4PEA==");
        let mut tag = decode("2IRK7xud3vnzIqqV+C2psg==");
        tag[0] ^= 0x01;
        let ct = decode(
            "nKf1tb4pHdnOo1AteNoeKgArGDg4EpahJYCnk0N2iocI1Mpd813L5bkKew5u19voZt8ICMLZdGHXOjpv7Bp+8TqcLcvhYWQ8+PyV",
        );

        let err = decrypt_factory_payload(&key(), &iv, &tag, &ct).unwrap_err();
        assert_eq!(err, "cred.decrypt_key_mismatch");
    }

    /// 用错误的密钥解同一份密文也必须被 tag 校验拦下。
    #[test]
    fn wrong_key_is_rejected() {
        let iv = decode("AQIDBAUGBwgJCgsMDQ4PEA==");
        let tag = decode("2IRK7xud3vnzIqqV+C2psg==");
        let ct = decode(
            "nKf1tb4pHdnOo1AteNoeKgArGDg4EpahJYCnk0N2iocI1Mpd813L5bkKew5u19voZt8ICMLZdGHXOjpv7Bp+8TqcLcvhYWQ8+PyV",
        );

        let mut bad_key = key();
        bad_key[31] ^= 0xff;

        assert_eq!(
            decrypt_factory_payload(&bad_key, &iv, &tag, &ct).unwrap_err(),
            "cred.decrypt_key_mismatch"
        );
    }

    /// 密文被改动同样必须报错（GCM 的完整性保证，不只是密钥正确性）。
    #[test]
    fn tampered_ciphertext_is_rejected() {
        let iv = decode("AQIDBAUGBwgJCgsMDQ4PEA==");
        let tag = decode("2IRK7xud3vnzIqqV+C2psg==");
        let mut ct = decode(
            "nKf1tb4pHdnOo1AteNoeKgArGDg4EpahJYCnk0N2iocI1Mpd813L5bkKew5u19voZt8ICMLZdGHXOjpv7Bp+8TqcLcvhYWQ8+PyV",
        );
        ct[0] ^= 0x80;

        assert_eq!(
            decrypt_factory_payload(&key(), &iv, &tag, &ct).unwrap_err(),
            "cred.decrypt_key_mismatch"
        );
    }

    /// tag 长度不是 16 字节时要报「格式非法」而不是静默接受，
    /// 否则短 tag 会被当成前缀比较而降低认证强度。
    #[test]
    fn short_tag_is_a_format_error() {
        let iv = decode("AQIDBAUGBwgJCgsMDQ4PEA==");
        let tag = vec![0u8; 8];

        let err = decrypt_factory_payload(&key(), &iv, &tag, b"x").unwrap_err();
        assert!(err.contains("authTag must be 16 bytes"), "got: {err}");
    }

    /// IV 长度既不是 12/16/32 也不足 16 时必须拒绝：
    /// 少于 16 字节无法凑满 GHASH 的第一个块，继续算就是读越界。
    #[test]
    fn unsupported_iv_length_is_rejected() {
        let short_iv = vec![0u8; 8];
        let tag = vec![0u8; 16];

        let err = decrypt_factory_payload(&key(), &short_iv, &tag, b"x").unwrap_err();
        assert!(err.contains("unsupported iv length"), "got: {err}");
    }

    /// key 长度不对时 `Aes256::new_from_slice` 必须报错而不是凑合。
    #[test]
    fn wrong_key_length_is_rejected() {
        let iv = decode("AQIDBAUGBwgJCgsMDQ4PEA==");
        let tag = vec![0u8; 16];

        assert!(decrypt_factory_payload(&[0u8; 16], &iv, &tag, b"x").is_err());
    }

    /// SQLite 的 value 列在不同 VS Code 版本里类型会变，
    /// 必须逐类型都能转成字符串，而不是只认 TEXT。
    #[test]
    fn sqlite_values_convert_by_type() {
        use rusqlite::types::Value as SqlValue;

        assert_eq!(
            sqlite_value_to_string(SqlValue::Text("token".into())),
            Some("token".to_string())
        );
        assert_eq!(
            sqlite_value_to_string(SqlValue::Integer(42)),
            Some("42".to_string())
        );
        assert_eq!(
            sqlite_value_to_string(SqlValue::Real(1.5)),
            Some("1.5".to_string())
        );
        assert_eq!(
            sqlite_value_to_string(SqlValue::Blob(b"blob".to_vec())),
            Some("blob".to_string())
        );
        // NULL 表示键存在但无值，以及非 UTF-8 的 BLOB，都应视为「读不到」
        assert_eq!(sqlite_value_to_string(SqlValue::Null), None);
        assert_eq!(sqlite_value_to_string(SqlValue::Blob(vec![0xff, 0xfe])), None);
    }
}
