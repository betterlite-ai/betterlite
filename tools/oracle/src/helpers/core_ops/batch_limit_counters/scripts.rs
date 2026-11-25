use betterlite_core::api::batch_enqueued_reserve::{BatchEnqueuedTokenScope, ReservationBackend};
use betterlite_core::api::batch_limit_counters::records::{BatchCounterResponse, WindowStartValue};
use betterlite_core::api::batch_limit_descriptors::descriptors::{
    BatchDescriptorIncrements, BatchLimitDescriptor,
};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum CachedWindow {
    Integer(i64),
    Float(f64),
    Text(String),
}

impl CachedWindow {
    pub(super) fn value(self) -> WindowStartValue {
        match self {
            Self::Integer(seconds) => WindowStartValue::Integer(seconds),
            Self::Float(seconds) => WindowStartValue::Float(seconds),
            Self::Text(text) => WindowStartValue::Text(text),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BackendName {
    Redis,
    Memory,
}

impl BackendName {
    pub(super) const fn reserved(self) -> ReservationBackend {
        match self {
            Self::Redis => ReservationBackend::Redis,
            Self::Memory => ReservationBackend::Memory,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum StoreReply {
    Reservation {
        #[serde(default)]
        backend: Option<BackendName>,
        #[serde(default)]
        owner: Option<String>,
    },
    OverLimit {
        scope: BatchEnqueuedTokenScope,
        enqueued: i64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoreScript {
    #[serde(default)]
    pub(super) replies: Vec<StoreReply>,
}

// one project counter's charge as a case scripts it, mapped to the core increment type on the way in
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IncrementScript {
    pub(super) requests: i64,
    pub(super) tokens: i64,
}

impl From<IncrementScript> for BatchDescriptorIncrements {
    fn from(increment: IncrementScript) -> Self {
        Self {
            requests: increment.requests,
            tokens: increment.tokens,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectScript {
    #[serde(default)]
    pub(super) descriptors: Vec<BatchLimitDescriptor>,
    #[serde(default)]
    pub(super) increments: Vec<IncrementScript>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LimiterScript {
    #[serde(default)]
    pub(super) base: Vec<BatchLimitDescriptor>,
    pub(super) project: ProjectScript,
    #[serde(default)]
    pub(super) responses: Vec<BatchCounterResponse>,
}
