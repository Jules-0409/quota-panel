pub mod commands;
pub mod credentials;
pub mod cursor;
pub mod devin;
pub mod factory;
pub mod http;
pub mod models;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};
use tokio::sync::Mutex;

use crate::models::{AppConfig, QuotaPayload, QuotaResults};

pub struct AppState {
    pub cached_quota: Mutex<Option<QuotaPayload>>,
    pub config: Mutex<AppConfig>,
    /// 界面语言，启动时由 UI 按 navigator.language 上报（set_locale）。
    /// 后端错误一律发语言无关的 key，翻译在 UI 做；这里只影响托盘菜单文案。
    pub locale: Mutex<String>,
    /// 三个 fetcher 共用的 HTTP client：`reqwest::Client` 内部是 Arc，clone 很便宜，
    /// 连接池和 TLS 配置只建一次。User-Agent 在这里统一设成如实报自己名字的全局值；
    /// 真正按数据源不同的头（如 Cursor 的 Cookie）才在各 fetcher 里按请求设置。
    pub client: reqwest::Client,
}

/// 托盘菜单 / tooltip 的文案，按语言取
pub(crate) fn tray_labels(locale: &str) -> (&'static str, &'static str, &'static str) {
    if locale == "zh" {
        (
            "立即刷新额度",
            "退出 Quota Panel",
            "Quota Panel - AI 额度监控",
        )
    } else {
        (
            "Refresh quotas now",
            "Quit Quota Panel",
            "Quota Panel - AI quota monitor",
        )
    }
}

pub(crate) fn build_tray_menu(
    app: &tauri::AppHandle,
    locale: &str,
) -> tauri::Result<Menu<tauri::Wry>> {
    let (refresh_txt, quit_txt, _) = tray_labels(locale);
    let quit_i = MenuItem::with_id(app, "quit", quit_txt, true, None::<&str>)?;
    let refresh_i = MenuItem::with_id(app, "refresh", refresh_txt, true, None::<&str>)?;
    Menu::with_items(app, &[&refresh_i, &quit_i])
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub(crate) async fn query_all_quotas(config: &AppConfig, client: &reqwest::Client) -> QuotaPayload {
    // 三个 fetcher 内部的阻塞部分各自跑在 spawn_blocking 里，这里是真的并发；
    // Cursor 那一路内部还会再并发拉 usage-summary 和 Grok Bot 的 GetSandUsageStatus
    let (factory_res, devin_res, cursor_res) = tokio::join!(
        crate::factory::fetch_factory(client),
        crate::devin::fetch_devin(client),
        crate::cursor::query_cursor_quota(client)
    );

    QuotaPayload {
        results: QuotaResults {
            factory: Some(factory_res),
            devin: Some(devin_res),
            cursor: Some(cursor_res),
        },
        config: config.clone(),
        at: now_millis(),
    }
}

/// CLI（`src/bin/quota.rs`）的入口：查一遍三个数据源，返回和 GUI 完全相同的结果。
///
/// 自己建 client、自己起一个多线程 runtime，完全不碰 Tauri 的 `AppHandle` /
/// `AppState`，所以命令行和 GUI 可以共用同一套 fetcher 而互不依赖。
pub async fn query_for_cli() -> QuotaResults {
    let client = reqwest::Client::builder()
        .user_agent(crate::http::USER_AGENT)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default();

    query_all_quotas(&AppConfig::default(), &client)
        .await
        .results
}

pub(crate) async fn do_refresh(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<QuotaPayload, String> {
    let _ = app.emit("refresh-start", ());
    let cfg = {
        let guard = state.config.lock().await;
        guard.clone()
    };

    let payload = query_all_quotas(&cfg, &state.client).await;

    {
        let mut cache = state.cached_quota.lock().await;
        *cache = Some(payload.clone());
    }

    let _ = app.emit("quota-updated", &payload);
    Ok(payload)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 只建一个共享 client，连接池 / TLS 配置全程复用。
    // UA 如实报自己，不伪造成任何厂商的客户端。
    let http_client = reqwest::Client::builder()
        .user_agent(crate::http::USER_AGENT)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default();

    let state = Arc::new(AppState {
        cached_quota: Mutex::new(None),
        config: Mutex::new(AppConfig::default()),
        locale: Mutex::new("en".into()),
        client: http_client,
    });

    let state_for_setup = state.clone();

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::get_quota,
            commands::refresh_quota,
            commands::get_config,
            commands::resize_window,
            commands::set_locale
        ])
        .setup(move |app| {
            // Setup system tray menu（初始英文，UI 启动后按系统语言 set_locale 重建）
            let menu = build_tray_menu(app.handle(), "en")?;

            let icon = app.default_window_icon().cloned();

            let mut tray_builder = TrayIconBuilder::with_id("main-tray")
                .menu(&menu)
                .tooltip(tray_labels("en").2)
                .show_menu_on_left_click(false);

            if let Some(ic) = icon {
                tray_builder = tray_builder.icon(ic);
            }

            let state_for_tray = state_for_setup.clone();

            tray_builder
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "quit" => {
                        app.exit(0);
                    }
                    "refresh" => {
                        let h = app.clone();
                        let s = state_for_tray.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = do_refresh(&h, &s).await;
                        });
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // 后台轮询：间隔取自配置 AppConfig::refresh_minutes（默认 5 分钟）
            let app_handle_for_poll = app.handle().clone();
            let state_for_poll = state_for_setup.clone();

            tauri::async_runtime::spawn(async move {
                // Initial fetch
                let _ = do_refresh(&app_handle_for_poll, &state_for_poll).await;

                loop {
                    // 每轮重新读一次配置，改了下一轮就生效
                    let refresh_minutes = {
                        let guard = state_for_poll.config.lock().await;
                        guard.refresh_minutes
                    };
                    // 至少 1 分钟：refresh_minutes 为 0 时不能变成打接口的死循环
                    let delay_secs = refresh_minutes.max(1).saturating_mul(60);
                    tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
                    let _ = do_refresh(&app_handle_for_poll, &state_for_poll).await;
                }
            });

            // Configure macOS window behavior
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_always_on_top(true);
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running quota-panel tauri application");
}
