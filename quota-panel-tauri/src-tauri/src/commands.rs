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

#[tauri::command]
pub async fn resize_window(
    window: WebviewWindow,
    width: f64,
    height: f64,
) -> Result<(), String> {
    // 保持右上角贴边锚定：当窗口宽度变大时，向左侧延伸，右边缘位置保持不变
    let scale_factor = window.scale_factor().unwrap_or(1.0);
    if let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) {
        let cur_logical_w = size.width as f64 / scale_factor;
        let diff_w = width - cur_logical_w;
        if diff_w.abs() > 1.0 {
            let cur_logical_x = pos.x as f64 / scale_factor;
            let new_logical_x = cur_logical_x - diff_w;
            let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition {
                x: new_logical_x,
                y: pos.y as f64 / scale_factor,
            }));
        }
    }

    window
        .set_size(tauri::Size::Logical(tauri::LogicalSize { width, height }))
        .map_err(|e| e.to_string())?;
    Ok(())
}
