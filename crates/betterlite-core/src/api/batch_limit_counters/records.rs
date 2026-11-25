use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_error::rate_limit::ProxyRateLimitError;
use indexmap::IndexMap;
use num_traits::ToPrimitive as _;
use serde::{Deserialize, Serialize};

use crate::api::batch_enqueued_reserve::BatchEnqueuedTokenScope;
use crate::api::batch_limit_descriptors::input::BatchModelUsage;

// the batch hook's clock is unix seconds, fractions included, as litellm's time_provider returns
pub type EpochClock = Arc<dyn Fn() -> f64 + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchCounterLimitType {
    Requests,
    Tokens,
    MaxParallelRequests,
}

impl BatchCounterLimitType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requests => "requests",
            Self::Tokens => "tokens",
            Self::MaxParallelRequests => "max_parallel_requests",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BatchCounterCode {
    Ok,
    OverLimit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchCounterStatus {
    pub code: BatchCounterCode,
    pub current_limit: i64,
    pub limit_remaining: i64,
    pub rate_limit_type: BatchCounterLimitType,
    pub descriptor_key: String,
    #[serde(default)]
    pub descriptor_value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchReservationBackend {
    // declaration order is the source's sorted() order, "local" before "redis"
    Local,
    Redis,
}

// derived Ord is the source's sort order, so this field order is load-bearing
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchReservationWindow {
    pub counter_key: String,
    pub window_start: String,
    pub backend: BatchReservationBackend,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchCounterResponse {
    pub overall_code: BatchCounterCode,
    pub statuses: Vec<BatchCounterStatus>,
    #[serde(default)]
    pub reservation_windows: BTreeSet<BatchReservationWindow>,
}

// B1042's runtime type carries no serde derives, so a case needs this record
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchEnqueuedLimitExceeded {
    pub scope: BatchEnqueuedTokenScope,
    pub enqueued: i64,
}

// a per-model row keeps absent keys distinct from zero, which the counter charge depends on
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchFileUsage {
    pub total_tokens: i64,
    pub request_count: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub per_model_usage: IndexMap<String, BTreeMap<String, i64>>,
}

impl BatchFileUsage {
    // _create_project_io_descriptors_for_models reads each row with .get(key, 0)
    #[must_use]
    pub fn project_usage(&self) -> IndexMap<String, BatchModelUsage> {
        self.per_model_usage
            .iter()
            .map(|(model, row)| {
                let usage = BatchModelUsage {
                    total_tokens: row.get("total_tokens").copied().unwrap_or(0),
                    output_tokens: row.get("output_tokens").copied().unwrap_or(0),
                };
                (model.clone(), usage)
            })
            .collect()
    }
}

// _WINDOW_START_ADAPTER's int | float | str under strict validation: only these three shapes reach int(float(...))
#[derive(Debug, Clone, PartialEq)]
pub enum WindowStartValue {
    Integer(i64),
    Float(f64),
    Text(String),
}

impl WindowStartValue {
    // int(float(window_start)): float is the gateway, so fractions truncate and a non-finite or huge value is absent
    #[must_use]
    pub fn seconds(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => value.to_f64()?.trunc().to_i64(),
            Self::Float(value) => value.trunc().to_i64(),
            Self::Text(text) => text.trim().parse::<f64>().ok()?.trunc().to_i64(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchTpdRefundOperation {
    pub key: String,
    pub increment_value: i64,
    pub ttl: i64,
    pub window_key: String,
    pub expected_window_start: String,
    pub reservation_backend: BatchReservationBackend,
}

#[derive(Debug)]
pub struct BatchLimitCountersResult {
    pub answers: Vec<BatchLimitCounterAnswer>,
}

#[derive(Debug)]
pub enum BatchLimitCounterAnswer {
    ReserveBatchEnqueuedTokens {
        rejection: Option<Box<ProxyRateLimitError>>,
    },
    RaiseEnqueuedLimitError {
        error: Box<ProxyRateLimitError>,
    },
    RaiseRateLimitError {
        error: Box<ProxyRateLimitError>,
    },
    CheckAndIncrementBatchCounters {
        rejection: Option<Box<ProxyRateLimitError>>,
    },
    ReadTpdWindowStart {
        window_start: Option<i64>,
    },
    BuildTpdRefundOps {
        operations: Vec<BatchTpdRefundOperation>,
    },
}

impl BatchLimitCounterAnswer {
    #[must_use]
    pub const fn is_rejection(&self) -> bool {
        match self {
            Self::ReserveBatchEnqueuedTokens { rejection: Some(_) }
            | Self::CheckAndIncrementBatchCounters { rejection: Some(_) }
            | Self::RaiseEnqueuedLimitError { .. }
            | Self::RaiseRateLimitError { .. } => true,
            Self::ReserveBatchEnqueuedTokens { rejection: None }
            | Self::CheckAndIncrementBatchCounters { rejection: None }
            | Self::ReadTpdWindowStart { .. }
            | Self::BuildTpdRefundOps { .. } => false,
        }
    }
}
