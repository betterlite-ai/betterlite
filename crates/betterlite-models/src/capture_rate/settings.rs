use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpendCaptureProvider {
    Openai,
}

// The general_settings.spend_capture_rate_check record; the loader has already applied every bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendCaptureRateCheckSettings {
    pub providers: Vec<SpendCaptureProvider>,
    pub threshold: Decimal,
    pub lookback_days: i64,
    pub openai_project_ids: Vec<String>,
}
