use std::sync::Arc;
use tauri::{AppHandle, State, WebviewWindow};

use crate::models::{AppConfig, QuotaPayload};
use crate::AppState;

#[tauri::command]
pub async fn get_quota(state: State<'_, Arc<AppState>>) -> Result<QuotaPayload, String> {
    let mut cache = state.cached_quota.lock().await;
    if let Some(existing) = &*cache {
        return Ok(existing.clone());
    }

    let cfg = {
        let guard = state.config.lock().await;
        guard.clone()
    };
    let payload = crate::query_all_quotas(&cfg, &state.client).await;
    *cache = Some(payload.clone());
    Ok(payload)
}

#[tauri::command]
pub async fn refresh_quota(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<QuotaPayload, String> {
    crate::do_refresh(&app, &state).await
}

#[tauri::command]
pub async fn get_config(state: State<'_, Arc<AppState>>) -> Result<AppConfig, String> {
    let guard = state.config.lock().await;
    Ok(guard.clone())
}

/// 保存设置（刷新间隔 + 两个颜色阈值）。
///
/// 先落盘再更新内存：写不进去就整体失败，免得界面显示「已保存」而下次启动又变回去。
/// 返回的是**收敛后**的配置，界面拿它回显，看到的一定是真正生效的值。
#[tauri::command]
pub async fn set_config(
    state: State<'_, Arc<AppState>>,
    config: AppConfig,
) -> Result<AppConfig, String> {
    let next = crate::config::sanitize(config);

    if let Some(path) = state.config_path.as_deref() {
        crate::config::save(path, &next).map_err(|e| {
            eprintln!("config: 写入 {} 失败: {e}", path.display());
            "config.save_failed".to_string()
        })?;
    }

    {
        let mut guard = state.config.lock().await;
        *guard = next.clone();
    }
    // 叫醒轮询循环：新的刷新间隔立刻开始计时
    state.config_changed.notify_one();

    Ok(next)
}

#[tauri::command]
pub async fn set_locale(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    locale: String,
) -> Result<(), String> {
    let norm = if locale.to_lowercase().starts_with("zh") {
        "zh"
    } else {
        "en"
    };
    {
        let mut guard = state.locale.lock().await;
        if *guard == norm {
            return Ok(());
        }
        *guard = norm.to_string();
    }

    if let Some(tray) = app.tray_by_id("main-tray") {
        let menu = crate::build_tray_menu(&app, norm).map_err(|e| e.to_string())?;
        let _ = tray.set_menu(Some(menu));
        let _ = tray.set_tooltip(Some(crate::tray_labels(norm).2));
    }
    Ok(())
}

#[tauri::command]
pub async fn resize_window(window: WebviewWindow, width: f64, height: f64) -> Result<(), String> {
    // 保持右上角贴边锚定：当窗口宽度变大时，向左侧延伸，右边缘位置保持不变
    let scale_factor = window.scale_factor().unwrap_or(1.0);
    let mut move_x = None;
    if let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) {
        let cur_logical_w = size.width as f64 / scale_factor;
        let diff_w = width - cur_logical_w;
        if diff_w.abs() > 1.0 {
            move_x = Some(pos.x as f64 / scale_factor - diff_w);
        }
        // 透明窗口上每一次多余的 setFrame 都是一次可见的重新合成，没变化就直接返回
        let cur_logical_h = size.height as f64 / scale_factor;
        if move_x.is_none()
            && (cur_logical_w - width).abs() < 0.5
            && (cur_logical_h - height).abs() < 0.5
        {
            return Ok(());
        }
    }

    if let Some(x) = move_x {
        let y = window
            .outer_position()
            .map(|p| p.y as f64 / scale_factor)
            .unwrap_or(0.0);
        let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
    }

    window
        .set_size(tauri::Size::Logical(tauri::LogicalSize { width, height }))
        .map_err(|e| e.to_string())?;
    Ok(())
}
