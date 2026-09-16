use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    pub used_percent: f64,
    pub seconds_remaining: Option<i64>,
    pub window_end: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FactoryWindows {
    pub standard: Option<HashMap<String, QuotaWindow>>,
    pub core: Option<HashMap<String, QuotaWindow>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FactoryQuota {
    pub ok: bool,
    pub id: String,
    pub name: String,
    pub plan_name: Option<String>,
    pub org_id: Option<String>,
    pub windows: FactoryWindows,
    pub overage_preference: Option<String>,
    pub extra_usage_balance_cents: Option<i64>,
    pub extra_usage_allowed: Option<bool>,
    pub fetched_at: i64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DevinQuota {
    pub ok: bool,
    pub id: String,
    pub name: String,
    pub plan_name: Option<String>,
    pub email: Option<String>,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub daily_remaining_percent: Option<f64>,
    pub weekly_remaining_percent: Option<f64>,
    pub daily_reset_at_unix: Option<i64>,
    pub weekly_reset_at_unix: Option<i64>,
    pub overage_balance_micros: Option<i64>,
    pub acu_consumed: Option<i64>,
    pub acu_limit: Option<i64>,
    pub plan_start_unix: Option<i64>,
    pub plan_end_unix: Option<i64>,
    pub source: Option<String>,
    pub stale: bool,
    pub stale_reason: Option<String>,
    pub fetched_at: i64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QuotaResults {
    pub factory: Option<FactoryQuota>,
    pub devin: Option<DevinQuota>,
    /// 含 Grok Bot 的周额度：Grok Bot 是 Cursor 的产品，两者共用同一份 Cursor 登录态
    pub cursor: Option<crate::cursor::CursorQuota>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub refresh_minutes: u64,
    pub warn_percent: f64,
    pub danger_percent: f64,
    pub always_on_top: bool,
    pub pinned: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            refresh_minutes: 5,
            warn_percent: 70.0,
            danger_percent: 90.0,
            always_on_top: true,
            pinned: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaPayload {
    pub results: QuotaResults,
    pub config: AppConfig,
    pub at: i64,
}
