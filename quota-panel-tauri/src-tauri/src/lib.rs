pub mod commands;
pub mod config;
pub mod credentials;
pub mod cursor;
pub mod devin;
pub mod factory;
pub mod http;
pub mod models;
#[cfg(target_os = "macos")]
pub mod tray_icon;

use std::path::PathBuf;
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
    /// 配置文件的落盘位置。拿不到应用配置目录时为 None：
    /// 这时设置只在内存里生效（重启回默认），不能因此起不来。
    pub config_path: Option<PathBuf>,
    /// 设置改动后叫醒轮询循环，让新的刷新间隔马上开始计时，
    /// 而不是等这一轮旧的 sleep 睡完（改间隔是用户能直接感知的操作）
    pub config_changed: tokio::sync::Notify,
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

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::get_quota,
            commands::refresh_quota,
            commands::get_config,
            commands::set_config,
            commands::resize_window,
            commands::set_locale
        ])
        .setup(move |app| {
            // 状态建在这里而不是外面：配置文件的目录要拿到 AppHandle 才问得出来。
            // 问不出来（权限异常等）就退化成「只在内存里生效」，不影响启动。
            let config_path = app
                .path()
                .app_config_dir()
                .ok()
                .map(|dir| crate::config::config_path(&dir));
            let config = config_path
                .as_deref()
                .map(crate::config::load)
                .unwrap_or_default();

            let state = Arc::new(AppState {
                cached_quota: Mutex::new(None),
                config: Mutex::new(config),
                config_path,
                config_changed: tokio::sync::Notify::new(),
                locale: Mutex::new("en".into()),
                client: http_client.clone(),
            });
            app.manage(state.clone());

            // Setup system tray menu（初始英文，UI 启动后按系统语言 set_locale 重建）
            let menu = build_tray_menu(app.handle(), "en")?;

            let mut tray_builder = TrayIconBuilder::with_id("main-tray")
                .menu(&menu)
                .tooltip(tray_labels("en").2)
                .show_menu_on_left_click(false);

            // macOS 的托盘用单色模板图，系统按菜单栏明暗自动上色；
            // 其他平台的托盘没有 template 机制，黑白色的模板图标在深色任务栏上等于隐形，
            // 所以还是用彩色 App 图标。
            #[cfg(target_os = "macos")]
            {
                let tray_icon = tauri::image::Image::new_owned(
                    crate::tray_icon::template_rgba(),
                    crate::tray_icon::SIZE,
                    crate::tray_icon::SIZE,
                );
                tray_builder = tray_builder.icon(tray_icon).icon_as_template(true);
            }
            #[cfg(not(target_os = "macos"))]
            {
                if let Some(ic) = app.default_window_icon().cloned() {
                    tray_builder = tray_builder.icon(ic);
                }
            }

            let state_for_tray = state.clone();

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
            let state_for_poll = state.clone();

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
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(delay_secs)) => {}
                        // 设置里改了间隔：重新计时，不用把旧的（可能很长的）间隔睡完
                        _ = state_for_poll.config_changed.notified() => continue,
                    }
                    let _ = do_refresh(&app_handle_for_poll, &state_for_poll).await;
                }
            });

            // Configure macOS window behavior
            #[cfg(target_os = "macos")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_always_on_top(true);
                }
                // 常驻托盘的小组件不该出现在 Dock 和 Cmd-Tab 里。手搓的 .app 靠
                // Info.plist 的 LSUIElement 做到，`cargo tauri build` 生成的 plist
                // 没有这一项，所以在这里钉死，两条打包路径的观感才一致。
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running quota-panel tauri application");
}
