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

    /// 固定文案的对照表。`QUOTA_LANG=en` 现在真的会全部走英文——
    /// 之前只有窗口名和错误文案分了语言，标题、单位、页脚还是写死的中文，
    /// 于是「英文模式」打出来的是中英混排。
    ///
    /// 写成表而不是 `match`：新增文案只加一行，和下面的 `error_text` 一致，
    /// 也不会被 rustfmt 拆成一处一屏的多行 `if`。
    fn t(&self, key: &str) -> String {
        let table: &[(&str, &str, &str)] = &[
            ("factory.standard", "Standard 用量", "Standard usage"),
            ("factory.core", "Droid Core (免费模型池)", "Droid Core (free model pool)"),
            ("factory.overage", "超额策略", "Overage policy"),
            ("factory.prepaid", "预付余额", "Prepaid balance"),
            ("devin.daily", "今日 已用", "Used today"),
            ("devin.weekly", "本周 已用", "Used this week"),
            ("devin.acu", "ACU", "ACU"),
            ("devin.overage", "超额余额", "Overage balance"),
            (
                "devin.source",
                "数据来自 Devin 服务端实时接口",
                "Source: Devin's live seat-management API",
            ),
            ("devin.renew_in", "下次续费 还有", "Renews in"),
            ("cursor.auto", "Auto 已用", "Auto used"),
            ("cursor.api", "API  已用", "API used"),
            ("cursor.requests", "请求", "Requests"),
            ("cursor.total", "综合用量", "Combined"),
            ("cursor.grok", "Grok Bot 周额度 已用", "Grok Bot weekly used"),
            ("cursor.cycle", "账期", "Billing cycle"),
            ("cursor.none", "没有取到任何数据", "No data available"),
            ("dur.reset", "已重置", "reset"),
            ("dur.reset_passed", "重置点已过", "reset time passed"),
            ("dur.resets_in", "重置于", "resets in"),
        ];

        match table.iter().find(|(k, _, _)| *k == key) {
            Some((_, zh_text, en_text)) => {
                if self.zh {
                    (*zh_text).to_string()
                } else {
                    (*en_text).to_string()
                }
            }
            // 未知 key 原样返回，方便发现拼错的 key
            None => key.to_string(),
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
            if (raw == *k || raw.starts_with(&format!("{k}: ")))
                && best.is_none_or(|(bk, _)| k.len() > bk.len())
            {
                best = Some((*k, if zh { *z } else { *e }));
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

/// 剩余秒数 → 人话。语言由调用方的 `Ctx` 决定。
fn human_duration(c: &Ctx, sec: i64) -> String {
    let zh = c.zh;
    if sec <= 0 {
        return c.t("dur.reset").to_string();
    }
    let d = sec / 86400;
    let h = (sec % 86400) / 3600;
    let m = (sec % 3600) / 60;
    if zh {
        if d > 0 {
            format!("{d}天{h}小时")
        } else if h > 0 {
            format!("{h}小时{m}分")
        } else {
            format!("{m}分")
        }
    } else if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

/// 重置时间点 → 还剩多久
fn reset_hint(c: &Ctx, at_unix: i64) -> String {
    let left = at_unix - now_unix();
    if left <= 0 {
        c.t("dur.reset_passed")
    } else if c.zh {
        // 中文习惯把「后」放在时长后面：重置于 3小时34分后
        format!("{} {} 后", c.t("dur.resets_in"), human_duration(c, left))
    } else {
        format!("{} {}", c.t("dur.resets_in"), human_duration(c, left))
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
        (c.t("factory.standard"), r.windows.standard.as_ref()),
        (c.t("factory.core"), r.windows.core.as_ref()),
    ];
    for (label, group) in groups {
        let Some(group) = group else { continue };
        println!("  {}", c.cyan(&label));
        for key in WINDOW_ORDER {
            let Some(w) = group.get(key) else { continue };
            let bar = text_bar(w.used_percent, 24);
            let reset = w
                .seconds_remaining
                .map(|s| c.dim(&format!("  {}", reset_hint(c, now_unix() + s))))
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
            "{} {} · {} ${:.2}",
            c.t("factory.overage"),
            policy,
            c.t("factory.prepaid"),
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
            .map(|t| c.dim(&format!("  {}", reset_hint(c, t))))
            .unwrap_or_default();
        println!(
            "    {} {}{}",
            c.t("devin.daily"),
            c.paint(severity_code(used), &text_bar(used, 24)),
            reset
        );
    }
    if let Some(remaining) = r.weekly_remaining_percent {
        let used = 100.0 - remaining;
        let reset = r
            .weekly_reset_at_unix
            .map(|t| c.dim(&format!("  {}", reset_hint(c, t))))
            .unwrap_or_default();
        println!(
            "    {} {}{}",
            c.t("devin.weekly"),
            c.paint(severity_code(used), &text_bar(used, 24)),
            reset
        );
    }
    if let (Some(consumed), Some(limit)) = (r.acu_consumed, r.acu_limit) {
        if limit > 0 {
            let used = (consumed as f64 / limit as f64) * 100.0;
            println!(
                "    {}  {}  {} / {}",
                c.t("devin.acu"),
                text_bar(used, 24),
                consumed,
                limit
            );
        }
    }
    if let Some(micros) = r.overage_balance_micros {
        if micros > 0 {
            println!(
                "    {}",
                c.dim(&format!(
                    "{} ${:.2}",
                    c.t("devin.overage"),
                    micros as f64 / 1e6
                ))
            );
        }
    }
    if let Some(end) = r.plan_end_unix {
        let left = end - now_unix();
        if left > 0 {
            println!(
                "    {}",
                c.dim(&format!(
                    "{} {} (unix {end})",
                    c.t("devin.renew_in"),
                    human_duration(c, left)
                ))
            );
        }
    }
    if let Some(src) = &r.source {
        println!(
            "  {}",
            c.dim(&format!("{} ({src})", c.t("devin.source")))
        );
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
                "    {} {}",
                c.t("cursor.auto"),
                c.paint(severity_code(p), &text_bar(p, 24))
            );
        }
        if let Some(p) = r.api_percent_used {
            println!(
                "    {} {}",
                c.t("cursor.api"),
                c.paint(severity_code(p), &text_bar(p, 24))
            );
        }
        if r.auto_percent_used.is_none() && r.api_percent_used.is_none() {
            if let (Some(used), Some(limit)) = (r.fast_requests_used, r.fast_requests_limit) {
                if limit > 0 {
                    let p = (used as f64 / limit as f64) * 100.0;
                    println!(
                        "    {} {}  {} / {}",
                        c.t("cursor.requests"),
                        c.paint(severity_code(p), &text_bar(p, 24)),
                        used,
                        limit
                    );
                }
            }
        }
        if let Some(p) = r.total_percent_used {
            println!(
                "    {}",
                c.dim(&format!("{} {:.1}%", c.t("cursor.total"), p))
            );
        }
    }

    if let Some(p) = r.grok_percent_used {
        printed_any = true;
        let reset = r
            .grok_reset_unix
            .map(|t| c.dim(&format!("  {}", reset_hint(c, t))))
            .unwrap_or_default();
        println!(
            "    {} {}{}",
            c.t("cursor.grok"),
            c.paint(severity_code(p), &text_bar(p, 24)),
            reset
        );
    }

    if let Some(reset_at) = r.cycle_reset_unix {
        println!(
            "    {}",
            c.dim(&format!("{} {}", c.t("cursor.cycle"), reset_hint(c, reset_at)))
        );
    }

    if let Some(err) = &r.error {
        println!("    {}", c.red(&format!("Cursor: {}", c.error_text(err))));
    }
    if let Some(err) = &r.grok_error {
        println!("    {}", c.red(&format!("Grok Bot: {}", c.error_text(err))));
    }

    if !printed_any && r.error.is_none() && r.grok_error.is_none() {
        println!("  {}", c.dim(&c.t("cursor.none")));
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
        let header = if c.zh {
            format!("刷新于 {} UTC  (Ctrl+C 退出)", fmt_utc_time(now_unix()))
        } else {
            format!("Refreshed at {} UTC  (Ctrl+C to quit)", fmt_utc_time(now_unix()))
        };
        println!("{}", c.dim(&header));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(zh: bool) -> Ctx {
        Ctx { tty: false, zh }
    }

    /// 进度条：填充块数量要和百分比对应，且总宽度固定。
    #[test]
    fn text_bar_fills_proportionally() {
        assert_eq!(text_bar(0.0, 10), "░░░░░░░░░░   0%");
        assert_eq!(text_bar(100.0, 10), "██████████ 100%");
        assert_eq!(text_bar(50.0, 10), "█████░░░░░  50%");
    }

    /// 越界百分比必须被夹住，否则 `"█".repeat(filled)` 会 panic 或画出超长条。
    #[test]
    fn text_bar_clamps_out_of_range() {
        // 负数与 >100 都不能 panic，长度也要保持在 width 以内
        let neg = text_bar(-50.0, 10);
        assert!(neg.contains("0%"), "got: {neg}");
        let over = text_bar(500.0, 10);
        assert!(over.contains("100%"), "got: {over}");
        // 填充部分不能超过 width
        assert_eq!(over.matches('█').count(), 10);
        assert_eq!(text_bar(-50.0, 10).matches('█').count(), 0);
    }

    /// 四舍五入：49.6% 在 width=10 时应当填 5 格而不是 4 格。
    #[test]
    fn text_bar_rounds_half_up() {
        assert_eq!(text_bar(49.6, 10).matches('█').count(), 5);
        assert_eq!(text_bar(44.0, 10).matches('█').count(), 4);
    }

    /// 严重程度分档：阈值本身算「已达」，边界必须落在正确一侧。
    #[test]
    fn severity_thresholds_are_inclusive() {
        assert_eq!(severity_code(0.0), "32"); // 绿
        assert_eq!(severity_code(69.9), "32");
        assert_eq!(severity_code(WARN_PERCENT), "33"); // 黄，阈值本身算黄
        assert_eq!(severity_code(89.9), "33");
        assert_eq!(severity_code(DANGER_PERCENT), "31"); // 红
        assert_eq!(severity_code(100.0), "31");
        assert_eq!(severity_code(1000.0), "31");
    }

    /// 人话时长：三种量级各测一次，中英都要有对应写法。
    #[test]
    fn human_duration_covers_all_magnitudes() {
        assert_eq!(human_duration(&ctx(true), 0), "已重置");
        assert_eq!(human_duration(&ctx(true), -10), "已重置");
        assert_eq!(human_duration(&ctx(true), 90), "1分");
        assert_eq!(human_duration(&ctx(true), 3700), "1小时1分");
        assert_eq!(human_duration(&ctx(true), 90000), "1天1小时");

        assert_eq!(human_duration(&ctx(false), 0), "reset");
        assert_eq!(human_duration(&ctx(false), 90), "1m");
        assert_eq!(human_duration(&ctx(false), 3700), "1h 1m");
        assert_eq!(human_duration(&ctx(false), 90000), "1d 1h");
    }

    /// 英文模式不能再漏中文：这是回归测试。
    /// 之前只有窗口名和错误文案分了语言，标题和单位是写死的中文。
    #[test]
    fn english_mode_has_no_chinese_leftovers() {
        let keys = [
            "factory.standard",
            "factory.core",
            "factory.overage",
            "factory.prepaid",
            "devin.daily",
            "devin.weekly",
            "devin.acu",
            "devin.overage",
            "devin.source",
            "devin.renew_in",
            "cursor.auto",
            "cursor.api",
            "cursor.requests",
            "cursor.total",
            "cursor.grok",
            "cursor.cycle",
            "cursor.none",
            "dur.reset",
            "dur.reset_passed",
            "dur.resets_in",
        ];
        let c = ctx(false);
        for key in keys {
            let s = c.t(key);
            assert!(
                !s.chars().any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch)),
                "key {key} 在英文模式下仍是中文: {s}"
            );
            assert_ne!(s, key, "key {key} 没有对应文案");
        }
        // 时长和重置提示同理
        for s in [
            human_duration(&c, 3700),
            human_duration(&c, 90000),
            reset_hint(&c, now_unix() + 3600),
        ] {
            assert!(
                !s.chars().any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch)),
                "英文模式下漏中文: {s}"
            );
        }
    }

    /// 中文模式反过来说应当有中文，避免两个分支被写成同一份。
    #[test]
    fn chinese_mode_actually_uses_chinese() {
        let c = ctx(true);
        assert_eq!(c.t("devin.daily"), "今日 已用");
        assert!(human_duration(&c, 90000).contains('天'));
        assert!(reset_hint(&c, now_unix() + 3600).contains("后"));
    }

    /// 未知 key 原样返回，方便暴露拼错的 key。
    #[test]
    fn unknown_label_key_returns_input() {
        assert_eq!(ctx(true).t("no.such.key"), "no.such.key");
    }

    /// 窗口 key → 显示名：已知的三种要翻译，未知的不能丢。
    #[test]
    fn window_labels_translate_known_keys() {
        assert_eq!(ctx(true).window_label("fiveHour"), "5 小时");
        assert_eq!(ctx(true).window_label("weekly"), "7 天");
        assert_eq!(ctx(true).window_label("monthly"), "30 天");
        assert_eq!(ctx(false).window_label("fiveHour"), "5h");
        // 接口以后新增窗口时原样显示，而不是丢掉
        assert_eq!(ctx(true).window_label("yearly"), "yearly");
    }

    /// UTC 时间格式：必须是 HH:MM:SS，且对同一天内是单调的。
    #[test]
    fn fmt_utc_time_is_zero_padded_clock() {
        assert_eq!(fmt_utc_time(0), "00:00:00");
        assert_eq!(fmt_utc_time(3661), "01:01:01");
        assert_eq!(fmt_utc_time(86399), "23:59:59");
        // 跨天回绕
        assert_eq!(fmt_utc_time(86400), "00:00:00");
    }

    /// 重置提示：已过期和未来两种情况。
    #[test]
    fn reset_hint_distinguishes_past_and_future() {
        let c = ctx(true);
        assert_eq!(reset_hint(&c, 0), "重置点已过");
        let future = reset_hint(&c, now_unix() + 7200);
        assert!(future.starts_with("重置于"), "got: {future}");
        assert!(future.ends_with('后'), "got: {future}");
    }

    /// 错误 key 的翻译：中英都要有，且未知 key 原样返回。
    #[test]
    fn error_text_translates_known_keys() {
        assert_eq!(
            ctx(true).error_text("devin.auth_expired"),
            "Devin 登录态已失效，请重新登录 Devin Desktop"
        );
        assert_eq!(
            ctx(false).error_text("devin.auth_expired"),
            "Devin session expired — sign in to Devin Desktop again"
        );
        assert_eq!(ctx(true).error_text("totally.unknown"), "totally.unknown");
    }

    /// 错误 key 后面跟的细节要原样附上，方便排查。
    #[test]
    fn error_text_keeps_trailing_detail() {
        let out = ctx(false).error_text("devin.request_failed: connection reset");
        assert!(out.starts_with("network request failed"), "got: {out}");
        assert!(out.contains("connection reset"), "got: {out}");
    }

    /// 最长前缀优先：`cursor.net_failed` 不能被更短的 key 抢先匹配。
    #[test]
    fn error_text_prefers_longest_key() {
        let out = ctx(false).error_text("cursor.no_token_db: db locked");
        assert!(out.contains("state.vscdb"), "got: {out}");
        assert!(out.contains("db locked"), "got: {out}");
    }

    /// 退出码判定：任一数据源成功就不算全失败。
    #[test]
    fn all_failed_only_when_every_target_failed() {
        let none = QuotaResults::default();
        assert!(all_failed(&none, &["factory", "devin", "cursor"]));

        let only_factory_ok = QuotaResults {
            factory: Some(FactoryQuota {
                ok: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!all_failed(&only_factory_ok, &["factory", "devin", "cursor"]));
        // 但如果只问 devin，那还是全失败
        assert!(all_failed(&only_factory_ok, &["devin"]));

        // ok=false 不等于成功
        let all_not_ok = QuotaResults {
            factory: Some(FactoryQuota {
                ok: false,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(all_failed(&all_not_ok, &["factory"]));
    }

    /// 请求了 unknown target 时不应影响判定（防御性）。
    #[test]
    fn all_failed_ignores_unknown_targets() {
        let results = QuotaResults::default();
        assert!(all_failed(&results, &["nope"]));
    }
}
