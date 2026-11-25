use std::collections::BTreeMap;

use betterlite_core::api::batch_enqueued_reserve::{
    BatchEnqueuedTokenReservation, BatchEnqueuedTokenScope, ReservationBackend,
};
use betterlite_core::api::batch_limit_counters::records::{
    BatchLimitCounterAnswer, BatchTpdRefundOperation,
};
use betterlite_core::api::batch_limit_descriptors::descriptors::{
    BatchDescriptorIncrements, BatchLimitDescriptor,
};
use betterlite_error::rate_limit::{ProxyRateLimitError, RateLimitCategory, RateLimitDimension};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub(super) struct StoreCallView {
    pub(super) tokens: i64,
    pub(super) scopes: Vec<BatchEnqueuedTokenScope>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct LimiterCallView {
    pub(super) descriptors: Vec<BatchLimitDescriptor>,
    pub(super) increments: Vec<BatchDescriptorIncrements>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ReservationView {
    tokens: i64,
    scopes: Vec<BatchEnqueuedTokenScope>,
    backend: ReservationBackend,
    owner: String,
}

impl From<&BatchEnqueuedTokenReservation> for ReservationView {
    fn from(reservation: &BatchEnqueuedTokenReservation) -> Self {
        Self {
            tokens: reservation.tokens,
            scopes: reservation.scopes.clone(),
            backend: reservation.backend,
            owner: reservation.owner.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ErrorView {
    status: u16,
    headers: BTreeMap<String, String>,
    category: RateLimitCategory,
    rate_limit_type: Option<RateLimitDimension>,
    model: String,
    llm_provider: String,
}

impl From<&ProxyRateLimitError> for ErrorView {
    fn from(error: &ProxyRateLimitError) -> Self {
        Self {
            status: 429,
            headers: header_values(error),
            category: error.category,
            rate_limit_type: error.rate_limit_type,
            model: error.model.clone(),
            llm_provider: error.llm_provider.clone(),
        }
    }
}

fn header_values(error: &ProxyRateLimitError) -> BTreeMap<String, String> {
    error
        .headers
        .as_ref()
        .map_or_else(BTreeMap::new, |present| {
            present
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        String::from_utf8_lossy(value.as_bytes()).into_owned(),
                    )
                })
                .collect()
        })
}

#[derive(Debug, Serialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(super) enum AnswerView {
    ReserveBatchEnqueuedTokens {
        rejection: Option<ErrorView>,
    },
    RaiseEnqueuedLimitError {
        error: ErrorView,
    },
    RaiseRateLimitError {
        error: ErrorView,
    },
    CheckAndIncrementBatchCounters {
        rejection: Option<ErrorView>,
    },
    ReadTpdWindowStart {
        window_start: Option<i64>,
    },
    BuildTpdRefundOps {
        operations: Vec<BatchTpdRefundOperation>,
    },
}

impl From<&BatchLimitCounterAnswer> for AnswerView {
    fn from(answer: &BatchLimitCounterAnswer) -> Self {
        match answer {
            BatchLimitCounterAnswer::ReserveBatchEnqueuedTokens { rejection } => {
                Self::ReserveBatchEnqueuedTokens {
                    rejection: rejection.as_deref().map(ErrorView::from),
                }
            }
            BatchLimitCounterAnswer::RaiseEnqueuedLimitError { error } => {
                Self::RaiseEnqueuedLimitError {
                    error: ErrorView::from(error.as_ref()),
                }
            }
            BatchLimitCounterAnswer::RaiseRateLimitError { error } => Self::RaiseRateLimitError {
                error: ErrorView::from(error.as_ref()),
            },
            BatchLimitCounterAnswer::CheckAndIncrementBatchCounters { rejection } => {
                Self::CheckAndIncrementBatchCounters {
                    rejection: rejection.as_deref().map(ErrorView::from),
                }
            }
            BatchLimitCounterAnswer::ReadTpdWindowStart { window_start } => {
                Self::ReadTpdWindowStart {
                    window_start: *window_start,
                }
            }
            BatchLimitCounterAnswer::BuildTpdRefundOps { operations } => Self::BuildTpdRefundOps {
                operations: operations.clone(),
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct EffectsView {
    pub(super) store_calls: Vec<StoreCallView>,
    pub(super) limiter_calls: Vec<LimiterCallView>,
    pub(super) base_calls: usize,
    pub(super) project_calls: usize,
    pub(super) cache_reads: Vec<String>,
    pub(super) reservation: Option<ReservationView>,
    pub(super) refund_ops: Vec<BatchTpdRefundOperation>,
}

#[derive(Debug, Serialize)]
pub(super) struct HelperOutput {
    pub(super) answers: Vec<AnswerView>,
    pub(super) effects: EffectsView,
}
