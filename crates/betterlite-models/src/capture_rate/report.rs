use rust_decimal::Decimal;
use serde::Serialize;

use super::settings::SpendCaptureProvider;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaptureRateDay {
    pub date: String,
    #[serde(serialize_with = "crate::capture_rate::format::serialize_money")]
    pub captured_spend: Decimal,
    #[serde(serialize_with = "crate::capture_rate::format::serialize_money")]
    pub provider_spend: Decimal,
    #[serde(serialize_with = "crate::capture_rate::format::serialize_optional_money")]
    pub capture_rate: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaptureRateReport {
    pub provider: SpendCaptureProvider,
    pub start_date: String,
    pub end_date: String,
    #[serde(serialize_with = "crate::capture_rate::format::serialize_money")]
    pub captured_spend: Decimal,
    #[serde(serialize_with = "crate::capture_rate::format::serialize_money")]
    pub provider_spend: Decimal,
    #[serde(serialize_with = "crate::capture_rate::format::serialize_optional_money")]
    pub capture_rate: Option<Decimal>,
    #[serde(serialize_with = "crate::capture_rate::format::serialize_money")]
    pub threshold: Decimal,
    pub below_threshold: bool,
    pub days: Vec<CaptureRateDay>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    Report(CaptureRateReport),
    CredentialMissing {
        provider: SpendCaptureProvider,
        env_var: String,
    },
    RequestFailed {
        provider: SpendCaptureProvider,
        detail: String,
    },
}

impl CheckResult {
    #[must_use]
    pub const fn provider(&self) -> SpendCaptureProvider {
        match self {
            Self::Report(report) => report.provider,
            Self::CredentialMissing { provider, .. } | Self::RequestFailed { provider, .. } => {
                *provider
            }
        }
    }

    // The gauge value: the rate, or None when the window produced no rate.
    pub const fn published_rate(&self) -> Option<Decimal> {
        match self {
            Self::Report(report) => report.capture_rate,
            Self::CredentialMissing { .. } | Self::RequestFailed { .. } => None,
        }
    }
}
