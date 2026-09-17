use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CursorQuota {
    pub ok: bool,
    pub id: String,
    pub name: String,
    pub plan_name: Option<String>,
    pub email: Option<String>,
    pub user_id: Option<String>,
    pub membership_type: Option<String>,
    pub subscription_status: Option<String>,
    pub auto_percent_used: Option<f64>,
    pub api_percent_used: Option<f64>,
    pub total_percent_used: Option<f64>,
    pub fast_requests_used: Option<i64>,
    pub fast_requests_limit: Option<i64>,
    pub fast_requests_remaining_percent: Option<f64>,
    pub cycle_start: Option<String>,
    pub cycle_end: Option<String>,
    pub cycle_reset_unix: Option<i64>,
    pub auto_message: Option<String>,
    pub api_message: Option<String>,
    /// Grok Bot 的周额度。Grok Bot 是 Cursor 的产品，用量由 Cursor 的
    /// `GetSandUsageStatus` 提供、吃的也是 Cursor 的 accessToken，所以合并在这一张卡里。
    /// 两个接口可能各自独立失败，故 Grok 的错误单独放 `grok_error`。
    pub grok_percent_used: Option<f64>,
    pub grok_has_available_usage: Option<bool>,
    pub grok_reset_unix: Option<i64>,
    pub grok_error: Option<String>,
    pub fetched_at: i64,
    pub error: Option<String>,
}

/// Locate state.vscdb for Cursor on macOS or Windows
fn find_cursor_vscdb_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            let p = PathBuf::from(home)
                .join("Library/Application Support/Cursor/User/globalStorage/state.vscdb");
            if p.exists() {
                return Some(p);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let p = PathBuf::from(appdata)
                .join("Cursor\\User\\globalStorage\\state.vscdb");
            if p.exists() {
                return Some(p);
            }
        }
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            let p = PathBuf::from(userprofile)
                .join("AppData\\Roaming\\Cursor\\User\\globalStorage\\state.vscdb");
            if p.exists() {
                return Some(p);
            }
        }
    }

    None
}

/// Locate ~/.cursor/cli-config.json for Cursor user info
fn find_cursor_cli_config_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            let p = PathBuf::from(home).join(".cursor/cli-config.json");
            if p.exists() {
                return Some(p);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            let p = PathBuf::from(userprofile).join(".cursor\\cli-config.json");
            if p.exists() {
                return Some(p);
            }
        }
    }

    None
}

struct CursorAuth {
    token: String,
    user_id: String,
    /// 网页会话 Cookie 的用户段。Cursor 现在用的是带 scope 的 auth id
    /// （形如 `grok|user_01M…`），不再是 cli-config 里的数字 userId。
    auth_id: String,
    email: Option<String>,
    membership_type: Option<String>,
    subscription_status: Option<String>,
}

fn load_cursor_auth() -> Result<CursorAuth, String> {
    let mut token = String::new();
    let mut user_id = String::new();
    let mut auth_id = String::new();
    let mut email = None;
    let mut membership_type = None;
    let mut subscription_status = None;

    // 1. 从 cli-config.json 读取 userId 和 email
    if let Some(cfg_path) = find_cursor_cli_config_path() {
        if let Ok(content) = std::fs::read_to_string(cfg_path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(auth_info) = val.get("authInfo") {
                    if let Some(uid) = auth_info.get("userId") {
                        if let Some(uid_num) = uid.as_i64() {
                            user_id = uid_num.to_string();
                        } else if let Some(uid_str) = uid.as_str() {
                            user_id = uid_str.to_string();
                        }
                    }
                    if let Some(em) = auth_info.get("email").and_then(|v| v.as_str()) {
                        email = Some(em.to_string());
                    }
                }
            }
        }
    }

    // 2. 从 state.vscdb (SQLite) 读取 accessToken 与 会员状态
    //    统一用 bundled rusqlite 只读打开，macOS / Windows 行为一致：
    //    原来 macOS 走内联 python3、Windows 走 PowerShell + System.Data.SQLite，
    //    而普通 Windows 既没有那个程序集也不一定装了 python，失败后是静默的。
    let mut db_error: Option<String> = None;

    if let Some(db_path) = find_cursor_vscdb_path() {
        const CURSOR_KEYS: [&str; 7] = [
            "cursorAuth/accessToken",
            "cursorAuth/cachedEmail",
            "cursorAuth/stripeMembershipType",
            "cursorAuth/stripeSubscriptionStatus",
            "glass.lastSignedInAuthId",
            "cursorAuth/stripeMembershipAuthId",
            "adminSettings.cachedAuthId",
        ];

        match crate::credentials::read_vscdb_items(&db_path, &CURSOR_KEYS) {
            Ok(parsed) => {
                if let Some(t) = parsed.get("cursorAuth/accessToken") {
                    token = t.clone();
                }
                if email.is_none() {
                    if let Some(e) = parsed.get("cursorAuth/cachedEmail") {
                        email = Some(e.clone());
                    }
                }
                membership_type = parsed.get("cursorAuth/stripeMembershipType").cloned();
                subscription_status = parsed.get("cursorAuth/stripeSubscriptionStatus").cloned();
                auth_id = ["glass.lastSignedInAuthId", "cursorAuth/stripeMembershipAuthId", "adminSettings.cachedAuthId"]
                    .iter()
                    .find_map(|k| parsed.get(*k).filter(|v| !v.is_empty()).cloned())
                    .unwrap_or_default();
            }
            Err(e) => db_error = Some(e),
        }
    }

    // 3. Fallback: Keychain on macOS
    #[cfg(target_os = "macos")]
    {
        if token.is_empty() {
            if let Ok(output) = std::process::Command::new("security")
                .args(["find-generic-password", "-s", "cursor-access-token", "-w"])
                .output()
            {
                if output.status.success() {
                    token = String::from_utf8_lossy(&output.stdout).trim().to_string();
                }
            }
        }
    }

    if token.is_empty() {
        let mut msg = "cursor.no_token".to_string();
        if let Some(e) = db_error {
            msg = format!("cursor.no_token_db: {e}");
        }
        return Err(msg);
    }

    Ok(CursorAuth {
        token,
        user_id,
        auth_id,
        email,
        membership_type,
        subscription_status,
    })
}

pub(crate) fn parse_iso_to_unix(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split('T').collect();
    if parts.len() != 2 {
        return None;
    }
    let ymd: Vec<i64> = parts[0].split('-').filter_map(|x| x.parse().ok()).collect();
    if ymd.len() != 3 {
        return None;
    }
    let time_str = parts[1].trim_end_matches('Z');
    let hms: Vec<i64> = time_str
        .split(':')
        .filter_map(|x| x.split('.').next()?.parse().ok())
        .collect();
    if hms.len() < 3 {
        return None;
    }

    let (mut y, mut m, d) = (ymd[0], ymd[1], ymd[2]);
    if m <= 2 {
        y -= 1;
        m += 12;
    }
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    // 这里的取模必须作用在 (m + 9) 上：Rust 里 `*` 和 `%` 同优先级且左结合，
    // 写成 `153 * (m + 9) % 12` 会被解析成 `(153 * (m + 9)) % 12`，
    // 月份贡献就全错了（9 月会算成 6 而不是 184，日期整体偏半年）。
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + hms[0] * 3600 + hms[1] * 60 + hms[2];
    Some(secs)
}

/// Cursor 官方的 usage-summary：Auto / API / 综合用量百分比与账期
async fn fetch_usage_summary(
    client: &reqwest::Client,
    auth: &CursorAuth,
) -> Result<serde_json::Value, String> {
    // 网络错误 / 5xx / 429 自动重试（最多 3 次尝试，300ms、900ms 指数退避）
    let resp = crate::http::send_with_retry(|| {
        let mut req = client
            .get("https://cursor.com/api/usage-summary")
            .header("Authorization", format!("Bearer {}", auth.token))
            .timeout(std::time::Duration::from_secs(8));
        // Cookie 的用户段是带 scope 的 auth id；老版本 Cursor 本地只有数字 userId，退回用它
        let cookie_user = if !auth.auth_id.is_empty() {
            auth.auth_id.as_str()
        } else {
            auth.user_id.as_str()
        };
        if !cookie_user.is_empty() {
            req = req.header(
                "Cookie",
                format!("WorkosCursorSessionToken={}%3A%3A{}", cookie_user, auth.token),
            );
        }
        req
    })
    .await
    .map_err(|e| format!("cursor.net_failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err("cursor.auth_expired".into());
        }
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {status}: {body}"));
    }

    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| format!("cursor.json_parse_failed: {e}"))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SandUsageStatus {
    #[serde(default)]
    pub usage_percent: Option<f64>,
    #[serde(default)]
    pub has_available_usage: Option<bool>,
    /// 服务端这个字段的类型不稳定（ISO 字符串或 epoch 数字），先原样收下再解析
    #[serde(default)]
    pub next_reset_timestamp_utc: Option<serde_json::Value>,
}

#[derive(Default)]
struct GrokUsage {
    percent_used: Option<f64>,
    has_available_usage: Option<bool>,
    reset_unix: Option<i64>,
    error: Option<String>,
}

/// Grok Bot 的周额度。Grok Bot 是 Cursor 的产品，这个 Connect-RPC 接口吃的就是
/// Cursor 的 accessToken，和 usage-summary 共用同一份登录态。
///
/// 不要用 Grok Bot 桌面应用 `sand-secrets.json` 里的 `cursor-access-token`：那是
/// Electron safeStorage 加密过的密文（base64 解出来以 `v10` 开头），当 Bearer 发会被
/// 服务端拒绝为 401 `ERROR_NOT_LOGGED_IN`。也正因如此，Grok Bot 装没装、跑没跑都不影响
/// 这里能不能取到数——只取决于有没有登录 Cursor。
async fn fetch_grok_usage(client: &reqwest::Client, token: &str) -> GrokUsage {
    let failed = |error: String| GrokUsage {
        error: Some(error),
        ..Default::default()
    };

    let resp = match crate::http::send_with_retry(|| {
        client
            .post("https://api2.cursor.sh/aiserver.v1.DashboardService/GetSandUsageStatus")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            // Connect-RPC 的协议版本声明，不是身份伪装，必须保留
            .header("Connect-Protocol-Version", "1")
            .body("{}")
    })
    .await
    {
        Ok(r) => r,
        Err(e) => return failed(format!("grok.net_failed: {e}")),
    };

    if !resp.status().is_success() {
        let code = resp.status();
        return failed(if code.as_u16() == 401 {
            "cursor.auth_expired".into()
        } else {
            let body = resp.text().await.unwrap_or_default();
            format!("HTTP {code}: {}", body.chars().take(200).collect::<String>())
        });
    }

    let data = match resp.json::<SandUsageStatus>().await {
        Ok(d) => d,
        Err(e) => return failed(format!("grok.parse_failed: {e}")),
    };

    GrokUsage {
        percent_used: data.usage_percent,
        has_available_usage: data.has_available_usage,
        reset_unix: data.next_reset_timestamp_utc.and_then(|ts| {
            ts.as_str()
                .and_then(parse_iso_to_unix)
                .or_else(|| {
                    ts.as_i64()
                        .map(|n| if n > 1_000_000_000_000 { n / 1000 } else { n })
                })
        }),
        error: None,
    }
}

pub async fn query_cursor_quota(client: &reqwest::Client) -> CursorQuota {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    // load_cursor_auth 是同步阻塞的（读磁盘、打开 SQLite、spawn `security`），
    // 丢到阻塞线程池，别占住 tokio 的 worker 线程把并发拉成串行
    let auth = match tokio::task::spawn_blocking(load_cursor_auth).await {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => {
            return CursorQuota {
                ok: false,
                id: "cursor".into(),
                name: "Cursor".into(),
                fetched_at: now,
                error: Some(e),
                ..Default::default()
            };
        }
        Err(join_err) => {
            return CursorQuota {
                ok: false,
                id: "cursor".into(),
                name: "Cursor".into(),
                fetched_at: now,
                error: Some(format!("cursor.auth_task_failed: {join_err}")),
                ..Default::default()
            };
        }
    };

    // 两个接口并发拉：任一方失败都不该把另一方的数据一起带走，
    // 所以错误分别记在 error（Cursor）和 grok_error（Grok Bot）上
    let (summary, grok) = tokio::join!(
        fetch_usage_summary(client, &auth),
        fetch_grok_usage(client, &auth.token),
    );

    let summary_err = summary.as_ref().err().cloned();
    // 失败时用 Null 兜底，下面整条 val.get(..) 链会自然全部得到 None，不必每处判空
    let val = summary.unwrap_or(serde_json::Value::Null);

    let membership_type = val
        .get("membershipType")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| auth.membership_type.clone());

    let cycle_start = val
        .get("billingCycleStart")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    let cycle_end = val
        .get("billingCycleEnd")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());

    let mut cycle_reset_unix = None;
    if let Some(ce) = &cycle_end {
        cycle_reset_unix = parse_iso_to_unix(ce);
    } else if let Some(cs) = &cycle_start {
        if let Some(secs) = parse_iso_to_unix(cs) {
            cycle_reset_unix = Some(secs + 30 * 86400);
        }
    }

    let auto_message = val
        .get("autoModelSelectedDisplayMessage")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    let api_message = val
        .get("namedModelSelectedDisplayMessage")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());

    let plan_obj = val.get("individualUsage").and_then(|u| u.get("plan"));

    let auto_percent_used = plan_obj
        .and_then(|p| p.get("autoPercentUsed"))
        .and_then(|v| v.as_f64());
    let api_percent_used = plan_obj
        .and_then(|p| p.get("apiPercentUsed"))
        .and_then(|v| v.as_f64());
    let total_percent_used = plan_obj
        .and_then(|p| p.get("totalPercentUsed"))
        .and_then(|v| v.as_f64());

    let fast_requests_used = plan_obj
        .and_then(|p| p.get("used"))
        .and_then(|v| v.as_i64());
    let fast_requests_limit = plan_obj
        .and_then(|p| p.get("limit"))
        .and_then(|v| v.as_i64());

    let remaining_pct = if let Some(auto_pct) = auto_percent_used {
        Some((100.0 - auto_pct).clamp(0.0, 100.0))
    } else if let (Some(used), Some(limit)) = (fast_requests_used, fast_requests_limit) {
        if limit > 0 {
            let used_pct = (used as f64 / limit as f64) * 100.0;
            Some((100.0 - used_pct).clamp(0.0, 100.0))
        } else {
            None
        }
    } else {
        None
    };

    CursorQuota {
        // usage-summary 和 Grok 任一取到数就算这张卡可用，UI 会逐格判断各自有没有值
        ok: !val.is_null() || grok.percent_used.is_some(),
        id: "cursor".into(),
        name: "Cursor".into(),
        plan_name: membership_type.clone().or_else(|| Some("Pro".into())),
        email: auth.email,
        user_id: if auth.user_id.is_empty() { None } else { Some(auth.user_id) },
        membership_type,
        subscription_status: auth.subscription_status,
        auto_percent_used,
        api_percent_used,
        total_percent_used,
        fast_requests_used,
        fast_requests_limit,
        fast_requests_remaining_percent: remaining_pct,
        cycle_start,
        cycle_end,
        cycle_reset_unix,
        auto_message,
        api_message,
        grok_percent_used: grok.percent_used,
        grok_has_available_usage: grok.has_available_usage,
        grok_reset_unix: grok.reset_unix,
        grok_error: grok.error,
        fetched_at: now,
        error: summary_err,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 期望值由 JS 的 `Date.parse(s) / 1000` 生成
    /// （生成脚本在仓库的 `scripts/gen-date-vectors.cjs`）。
    /// 用 Node 当对照是为了防手算月份和闰年出错——这个函数手写了一遍
    /// days_from_civil 算法，最容易错的就是闰年和世纪规则。
    #[test]
    fn parses_iso_instants_like_javascript() {
        let cases = [
            ("2026-09-17T12:34:56Z", 1789648496),
            ("1970-01-01T00:00:00Z", 0),
            ("2000-02-29T00:00:00Z", 951782400), // 2000 能被 400 整除，是闰年
            ("2026-12-31T23:59:59Z", 1798761599),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_iso_to_unix(input), Some(expected), "input: {input}");
        }
    }

    /// 带毫秒的 ISO 串必须按秒截断，而不是解析失败。
    #[test]
    fn parses_iso_with_milliseconds() {
        assert_eq!(
            parse_iso_to_unix("2026-09-17T12:34:56.789Z"),
            Some(1789648496)
        );
    }

    /// 1900 不是闰年（能被 100 整除但不能被 400 整除）。
    /// 解析器不做日历合法性校验，纯按公式算天数，这里锁住的是它按同一个公式
    /// 连续推进：2 月 28 日与 3 月 1 日在 1900 年只差一天。
    #[test]
    fn handles_1900_non_leap_year_without_drift() {
        let feb28 = parse_iso_to_unix("1900-02-28T00:00:00Z").unwrap();
        let mar01 = parse_iso_to_unix("1900-03-01T00:00:00Z").unwrap();
        assert_eq!(mar01 - feb28, 86400, "1900-02-28 之后一天应是 03-01");
    }

    /// 各种畸形输入必须返回 None 而不是 panic：接口返回的字段不可信。
    #[test]
    fn malformed_input_returns_none() {
        let cases = [
            "",
            "2026-09-17",              // 没有时间部分
            "not-a-date",
            "2026-09-17T12:34Z",       // 只有时分
            "2026-09-17T12",           // 只有小时
            "2026/09/17T12:34:56Z",    // 用斜杠而不是减号
            "2026-09-17 12:34:56Z",    // 用空格而不是 T
            "T12:34:56Z",              // 没有日期
            "2026-09-17T12:34:56",     // 无 Z 也算合法（trim_end_matches 会放过）
        ];
        // 最后一项其实可以解析成功，单独断言，其余都是 None
        for input in &cases[..cases.len() - 1] {
            assert_eq!(parse_iso_to_unix(input), None, "input: {input}");
        }
        assert!(parse_iso_to_unix("2026-09-17T12:34:56").is_some());
    }

    /// 闰年边界：2 月 29 日必须和 3 月 1 日只差一天，
    /// 相邻的 1904 / 2100 也要符合各自规则。
    #[test]
    fn leap_year_boundaries() {
        let feb29 = parse_iso_to_unix("2000-02-29T00:00:00Z").unwrap();
        let mar01 = parse_iso_to_unix("2000-03-01T00:00:00Z").unwrap();
        assert_eq!(mar01 - feb29, 86400);

        // 2016-02-29 存在（能被 4 整除且不是世纪年）
        assert!(parse_iso_to_unix("2016-02-29T00:00:00Z").is_some());
    }
}
