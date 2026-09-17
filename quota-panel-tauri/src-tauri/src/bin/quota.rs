//! `quota` —— 命令行额度查询，与 GUI 共用同一套 fetcher（`query_for_cli`）。
//!
//! 用法：
//!   quota                 三个数据源
//!   quota factory         只看 Factory
//!   quota devin           只看 Devin
//!   quota cursor          只看 Cursor（含 Grok Bot）
//!   quota --json          机器可读
//!   quota watch           每 60 秒刷新（Ctrl+C 退出）
//!
//! Windows 上这个 bin 故意**不**声明 `windows_subsystem = "windows"`：
//! 主程序是 GUI 子系统、本身没有控制台，直接跑是打印不出东西的，
//! 所以命令行必须是一个独立的控制台程序。

use std::io::IsTerminal;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use quota_panel_lib::cursor::CursorQuota;
use quota_panel_lib::models::{DevinQuota, FactoryQuota, QuotaResults};

/// Factory 的窗口固定按这个顺序打印（HashMap 本身无序，直接遍历每次都不一样）
const WINDOW_ORDER: [&str; 3] = ["fiveHour", "weekly", "monthly"];

/// 警告 / 危险的默认阈值，与 `models.rs` 里 `AppConfig` 的默认值一致
const WARN_PERCENT: f64 = 70.0;
const DANGER_PERCENT: f64 = 90.0;

struct Ctx {
    tty: bool,
    zh: bool,
}

impl Ctx {
    fn detect() -> Self {
        // 显式指定优先；否则看 POSIX 惯例的 locale 变量；都不是就跟着这台机器的
        // 系统语言习惯走中文（原 Electron 版 CLI 本来就是中文输出）
        let mut zh = true;
        for key in ["QUOTA_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(v) = std::env::var(key) {
                let v = v.to_lowercase();
                if v.starts_with("en") {
                    zh = false;
                    break;
                }
                if v.starts_with("zh") {
                    zh = true;
                    break;
                }
            }
        }
        Self {
            tty: std::io::stdout().is_terminal(),
            zh,
        }
    }

    fn paint(&self, code: &str, s: &str) -> String {
        if self.tty && std::env::var_os("NO_COLOR").is_none() {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }
    fn bold(&self, s: &str) -> String {
        self.paint("1", s)
    }
    fn cyan(&self, s: &str) -> String {
        self.paint("36", s)
    }
    fn red(&self, s: &str) -> String {
        self.paint("31", s)
    }

    /// 窗口 key → 显示名。未知 key 原样返回，接口以后加窗口也不会丢信息
    fn window_label(&self, key: &str) -> String {
        match (self.zh, key) {
            (true, "fiveHour") => "5 小时".into(),
            (true, "weekly") => "7 天".into(),
            (true, "monthly") => "30 天".into(),
            (false, "fiveHour") => "5h".into(),
            (false, "weekly") => "7d".into(),
            (false, "monthly") => "30d".into(),
            _ => key.to_string(),
        }
    }

    /// 后端发的是语言无关的 key（见 ui/index.html 的 ERROR_TEXT），这里翻译成人话
    fn error_text(&self, raw: &str) -> String {
        let zh = self.zh;
        let table: &[(&str, &str, &str)] = &[
            ("cred.sqlite_open", "无法只读打开本地 SQLite 凭据库", "cannot open the local SQLite credential db read-only"),
            ("cred.sqlite_readonly_mode", "无法把本地库设为只读查询模式", "cannot set the local db to read-only mode"),
            ("cred.sqlite_query", "查询本地凭据表失败", "query on the local credential table failed"),
            ("cred.key_read", "读取本地凭据键失败", "failed to read a local credential key"),
            ("cred.decrypt_key_mismatch", "解密失败：加密密钥不匹配", "decryption failed: encryption key mismatch"),
            ("cred.devin_windsurf_parse", "解析 Devin 登录态失败", "failed to parse the Devin session state"),
            ("devin.no_session", "没找到有效的 Devin 登录态或凭据", "no valid Devin session or credentials found"),
            ("devin.auth_expired", "Devin 登录态已失效，请重新登录 Devin Desktop", "Devin session expired — sign in to Devin Desktop again"),
            ("devin.credential_task_failed", "读取 Devin 凭据的任务失败", "the Devin credential task failed"),
            ("devin.request_failed", "网络请求失败", "network request failed"),
            ("devin.http_status", "Devin 接口返回异常状态", "Devin API returned an error status"),
            ("devin.body_read_failed", "读取 Devin 响应失败", "failed to read the Devin response"),
            ("factory.credential_task_failed", "读取 Factory 凭据的任务失败", "the Factory credential task failed"),
            ("factory.auth_expired", "Factory 登录态已失效，请重新登录 droid", "Factory session expired — sign in to droid again"),
            ("cursor.no_token", "未检测到 Cursor Access Token，请确认已在 Cursor 中登录", "no Cursor access token found — make sure you are signed in to Cursor"),
            ("cursor.no_token_db", "读取 Cursor state.vscdb 失败", "failed to read Cursor state.vscdb"),
            ("cursor.net_failed", "网络请求失败", "network request failed"),
            ("cursor.json_parse_failed", "解析 JSON 响应失败", "failed to parse the JSON response"),
            ("cursor.auth_expired", "Cursor 登录态已失效，请在 Cursor 中重新登录", "Cursor session expired — sign in to Cursor again"),
            ("cursor.auth_task_failed", "读取 Cursor 登录态的任务失败", "the Cursor session read task failed"),
            ("grok.net_failed", "网络请求失败", "network request failed"),
            ("grok.parse_failed", "解析响应失败", "failed to parse the response"),
        ];

        // key 后面可能跟 `: 细节`，匹配最长的 key 前缀，细节原样附在后面
        let mut best: Option<(&str, &str)> = None;
        for (k, z, e) in table {
            if raw == *k || raw.starts_with(&format!("{k}: ")) {
                if best.map_or(true, |(bk, _)| k.len() > bk.len()) {
                    best = Some((*k, if zh { *z } else { *e }));
                }
            }
        }
        match best {
            Some((k, text)) => {
                let rest = raw[k.len()..].trim_start_matches(": ");
                if rest.is_empty() {
                    text.to_string()
                } else {
                    format!("{text} ({rest})")
                }
            }
            None => raw.to_string(),
        }
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// 剩余秒数 → 人话
fn human_duration(sec: i64) -> String {
    if sec <= 0 {
        return "已重置".into();
    }
    let d = sec / 86400;
    let h = (sec % 86400) / 3600;
    let m = (sec % 3600) / 60;
    if d > 0 {
        format!("{d}天{h}小时")
    } else if h > 0 {
        format!("{h}小时{m}分")
    } else {
        format!("{m}分")
    }
}

/// 重置时间点 → 还剩多久
fn reset_hint(at_unix: i64) -> String {
    let left = at_unix - now_unix();
    if left <= 0 {
        "重置点已过".into()
    } else {
        format!("重置于 {} 后", human_duration(left))
    }
}

/// 用量进度条（纯文本）
fn text_bar(percent: f64, width: usize) -> String {
    let p = percent.clamp(0.0, 100.0);
    let filled = ((p / 100.0) * width as f64).round() as usize;
    format!(
        "{}{} {:>3}%",
        "█".repeat(filled),
        "░".repeat(width.saturating_sub(filled)),
        p.round() as i64
    )
}

fn severity_code(used: f64) -> &'static str {
    if used >= DANGER_PERCENT {
        "31"
    } else if used >= WARN_PERCENT {
        "33"
    } else {
        "32"
    }
}

fn print_factory(c: &Ctx, r: &FactoryQuota) {
    println!("{}", c.bold("\nFactory (Droid)"));
    if !r.ok {
        let err = r.error.clone().unwrap_or_default();
        println!("  {}", c.red(&c.error_text(&err)));
        return;
    }
    if let Some(org) = &r.org_id {
        println!("  {}", c.dim(&format!("org {org}")));
    }

    let groups = [
        ("Standard 用量", r.windows.standard.as_ref()),
        ("Droid Core (免费模型池)", r.windows.core.as_ref()),
    ];
    for (label, group) in groups {
        let Some(group) = group else { continue };
        println!("  {}", c.cyan(label));
        for key in WINDOW_ORDER {
            let Some(w) = group.get(key) else { continue };
            let bar = text_bar(w.used_percent, 24);
            let reset = w
                .seconds_remaining
                .map(|s| c.dim(&format!("  {}", reset_hint(now_unix() + s))))
                .unwrap_or_default();
            println!(
                "    {:<6} {}{}",
                c.window_label(key),
                c.paint(severity_code(w.used_percent), &bar),
                reset
            );
        }
    }

    let cents = r.extra_usage_balance_cents.unwrap_or(0);
    let policy = r.overage_preference.clone().unwrap_or_else(|| "-".into());
    println!(
        "  {}",
        c.dim(&format!(
            "超额策略 {} · 预付余额 ${:.2}",
            policy,
            cents as f64 / 100.0
        ))
    );
}

fn print_devin(c: &Ctx, r: &DevinQuota) {
    println!("{}", c.bold("\nDevin"));
    if !r.ok {
        let err = r.error.clone().unwrap_or_default();
        println!("  {}", c.red(&c.error_text(&err)));
        return;
    }

    let who = [r.plan_name.clone(), r.email.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    if !who.is_empty() {
        println!("  {}", c.cyan(&who));
    }
    if let Some(team) = &r.team_id {
        println!("  {}", c.dim(team));
    }

    if let Some(remaining) = r.daily_remaining_percent {
        let used = 100.0 - remaining;
        let reset = r
            .daily_reset_at_unix
            .map(|t| c.dim(&format!("  {}", reset_hint(t))))
            .unwrap_or_default();
        println!(
            "    今日 已用 {}{}",
            c.paint(severity_code(used), &text_bar(used, 24)),
            reset
        );
    }
    if let Some(remaining) = r.weekly_remaining_percent {
        let used = 100.0 - remaining;
        let reset = r
            .weekly_reset_at_unix
            .map(|t| c.dim(&format!("  {}", reset_hint(t))))
            .unwrap_or_default();
        println!(
            "    本周 已用 {}{}",
            c.paint(severity_code(used), &text_bar(used, 24)),
            reset
        );
    }
    if let (Some(consumed), Some(limit)) = (r.acu_consumed, r.acu_limit) {
        if limit > 0 {
            let used = (consumed as f64 / limit as f64) * 100.0;
            println!(
                "    ACU  {}  {} / {}",
                text_bar(used, 24),
                consumed,
                limit
            );
        }
    }
    if let Some(micros) = r.overage_balance_micros {
        if micros > 0 {
            println!("    {}", c.dim(&format!("超额余额 ${:.2}", micros as f64 / 1e6)));
        }
    }
    if let Some(end) = r.plan_end_unix {
        let left = end - now_unix();
        if left > 0 {
            println!(
                "    {}",
                c.dim(&format!("下次续费 还有 {}（unix {end}）", human_duration(left)))
            );
        }
    }
    if let Some(src) = &r.source {
        println!("  {}", c.dim(&format!("数据来自 Devin 服务端实时接口（{src}）")));
    }
}

fn print_cursor(c: &Ctx, r: &CursorQuota) {
    println!("{}", c.bold("\nCursor"));

    // usage-summary 和 Grok 是两个独立接口，各自可能单独失败，
    // 所以「整张卡可用」不代表两格都有数：有值就打，没值就说为什么
    let mut printed_any = false;

    if r.auto_percent_used.is_some()
        || r.api_percent_used.is_some()
        || r.total_percent_used.is_some()
        || r.fast_requests_used.is_some()
    {
        printed_any = true;
        let who = [r.plan_name.clone(), r.email.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        if !who.is_empty() {
            println!("  {}", c.cyan(&who));
        }
        if let Some(p) = r.auto_percent_used {
            println!(
                "    Auto 已用 {}",
                c.paint(severity_code(p), &text_bar(p, 24))
            );
        }
        if let Some(p) = r.api_percent_used {
            println!(
                "    API  已用 {}",
                c.paint(severity_code(p), &text_bar(p, 24))
            );
        }
        if r.auto_percent_used.is_none() && r.api_percent_used.is_none() {
            if let (Some(used), Some(limit)) = (r.fast_requests_used, r.fast_requests_limit) {
                if limit > 0 {
                    let p = (used as f64 / limit as f64) * 100.0;
                    println!(
                        "    请求 {}  {} / {}",
                        c.paint(severity_code(p), &text_bar(p, 24)),
                        used,
                        limit
                    );
                }
            }
        }
        if let Some(p) = r.total_percent_used {
            println!("    {}", c.dim(&format!("综合用量 {:.1}%", p)));
        }
    }

    if let Some(p) = r.grok_percent_used {
        printed_any = true;
        let reset = r
            .grok_reset_unix
            .map(|t| c.dim(&format!("  {}", reset_hint(t))))
            .unwrap_or_default();
        println!(
            "    Grok Bot 周额度 已用 {}{}",
            c.paint(severity_code(p), &text_bar(p, 24)),
            reset
        );
    }

    if let Some(t) = r.cycle_reset_unix {
        println!("    {}", c.dim(&format!("账期 {}", reset_hint(t))));
    }

    if let Some(err) = &r.error {
        println!("    {}", c.red(&format!("Cursor: {}", c.error_text(err))));
    }
    if let Some(err) = &r.grok_error {
        println!("    {}", c.red(&format!("Grok Bot: {}", c.error_text(err))));
    }

    if !printed_any && r.error.is_none() && r.grok_error.is_none() {
        println!("  {}", c.dim("没有取到任何数据"));
    }
}

fn print_results(c: &Ctx, results: &QuotaResults, targets: &[&str]) {
    for id in targets {
        match *id {
            "factory" => {
                if let Some(r) = &results.factory {
                    print_factory(c, r);
                }
            }
            "devin" => {
                if let Some(r) = &results.devin {
                    print_devin(c, r);
                }
            }
            "cursor" => {
                if let Some(r) = &results.cursor {
                    print_cursor(c, r);
                }
            }
            _ => {}
        }
    }
}

/// 请求的目标是否全部失败（用于确定退出码，方便脚本判断）
fn all_failed(results: &QuotaResults, targets: &[&str]) -> bool {
    let mut any_ok = false;
    for id in targets {
        let ok = match *id {
            "factory" => results.factory.as_ref().map(|r| r.ok),
            "devin" => results.devin.as_ref().map(|r| r.ok),
            "cursor" => results.cursor.as_ref().map(|r| r.ok),
            _ => None,
        };
        if ok.unwrap_or(false) {
            any_ok = true;
        }
    }
    !any_ok
}

fn usage() {
    println!(
        "quota —— 查询 Factory / Devin / Cursor 的实时额度\n\n\
         用法:\n  \
           quota                 三个数据源\n  \
           quota factory         只看 Factory\n  \
           quota devin           只看 Devin\n  \
           quota cursor          只看 Cursor（含 Grok Bot）\n  \
           quota --json          机器可读输出\n  \
           quota watch           每 60 秒刷新\n\n\
         环境变量:\n  \
           QUOTA_LANG=en|zh      界面语言（默认跟随系统）\n  \
           NO_COLOR=1            关闭颜色\n"
    );
}

async fn once(c: &Ctx, targets: &[&str], json: bool) -> bool {
    let results = quota_panel_lib::query_for_cli().await;
    if json {
        match serde_json::to_string_pretty(&results) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("序列化失败: {e}");
                return false;
            }
        }
    } else {
        print_results(c, &results, targets);
        println!();
    }
    !all_failed(&results, targets)
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        usage();
        return;
    }

    let json = args.iter().any(|a| a == "--json");
    let watch = args.iter().any(|a| a == "watch");
    let only = args
        .iter()
        .find(|a| ["factory", "devin", "cursor", "all"].contains(&a.as_str()))
        .cloned()
        .unwrap_or_else(|| "all".into());

    // 不认识的参数直接报错，别静默忽略：打错字（`--jsn`、`cursro`）时
    // 静默降级成「查全部」比报错更糟——会让人以为看到的就是自己查的那一项
    for a in &args {
        let known = ["--json", "watch", "all", "factory", "devin", "cursor"];
        if !known.contains(&a.as_str()) {
            eprintln!("未知参数: {a}\n");
            usage();
            std::process::exit(2);
        }
    }

    let targets: Vec<&str> = if only == "all" {
        vec!["factory", "devin", "cursor"]
    } else {
        vec![match only.as_str() {
            "factory" => "factory",
            "devin" => "devin",
            _ => "cursor",
        }]
    };

    let c = Ctx::detect();

    if !watch {
        let ok = once(&c, &targets, json).await;
        std::process::exit(if ok { 0 } else { 1 });
    }

    // watch：定时刷新，写在同一屏
    let mut first = true;
    loop {
        if !first && c.tty {
            print!("\x1b[2J\x1b[H");
        }
        first = false;
        println!(
            "{}",
            c.dim(&format!(
                "刷新于 {} UTC  (Ctrl+C 退出)",
                fmt_utc_time(now_unix())
            ))
        );
        let _ = once(&c, &targets, json).await;
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

/// epoch 秒 → HH:MM:SS **UTC**。
///
/// 刻意只打 UTC 并标出来：标准库里拿本地时区偏移要额外依赖或平台 API，
/// 与其悄悄把一个 UTC 时间当成当地时间打出去，不如写明是 UTC。
fn fmt_utc_time(unix: i64) -> String {
    let secs = unix.rem_euclid(86400);
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}
