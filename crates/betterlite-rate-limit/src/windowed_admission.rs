pub mod admission;
pub mod batch_script;
pub mod collect;
pub mod over_limit;

mod tests;

use std::num::TryFromIntError;
use std::sync::Arc;
use std::time::SystemTimeError;

use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_redis::batch::error::BatchError;
use uuid::Uuid;

use crate::concurrency_gauges::ConcurrencyGauges;
use crate::concurrency_gauges::GaugeError;
use crate::fail_closed::{FailClosedPolicy, RateLimitUnverifiableError};
use crate::limiter_records::{CounterReplyError, LimiterClock, RateLimitDescriptor};
use crate::rate_limit_window_store::counter_value::CounterValueError;
use crate::rate_limit_window_store::{CounterScript, WindowStore, WindowStoreError};
use crate::settlement_and_rollback::SettlementError;

pub const OK_CODE: &str = "OK";
pub const OVER_LIMIT_CODE: &str = "OVER_LIMIT";

pub struct WindowedAdmission {
    pub window_size: i64,
    pub clock: Arc<dyn LimiterClock>,
    pub fail_closed: FailClosedPolicy,
    pub windows: Arc<WindowStore>,
    // LiteLLM's window_guarded_token_increment_script, which B3034's AtomicSettlement refunds through
    pub refund_script: Option<Arc<dyn CounterScript>>,
    pub gauges: Arc<ConcurrencyGauges>,
    pub slot_ids: Arc<dyn SlotIdGenerator>,
}

// LiteLLM's uuid.uuid4().hex: 32 lowercase hex characters, so a generated slot id never carries dashes
pub trait SlotIdGenerator: Send + Sync {
    fn next_slot_id(&self) -> String;
}

pub struct UuidSlotIds;

impl SlotIdGenerator for UuidSlotIds {
    fn next_slot_id(&self) -> String {
        Uuid::new_v4().simple().to_string()
    }
}

#[derive(Clone, Debug, Default)]
pub struct AdmissionRequest {
    pub descriptors: Vec<RateLimitDescriptor>,
    // the span B4293 and B4294 thread through DualCache calls, which betterlite's store and gauge calls do not take
    pub parent_otel_span: Option<ParentOtelSpan>,
    pub read_only: bool,
    pub skip_tpm_check: bool,
    pub parallel_slot_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum WindowedAdmissionError {
    #[error(transparent)]
    Unverifiable(#[from] RateLimitUnverifiableError),
    #[error("the limiter clock reads a time before the Unix epoch")]
    ClockBeforeEpoch(#[source] SystemTimeError),
    #[error("the limiter clock reads {value} seconds, which has no signed 64-bit count")]
    ClockOverflow {
        value: u64,
        #[source]
        source: TryFromIntError,
    },
    #[error(transparent)]
    WindowStore(#[from] WindowStoreError),
    #[error(transparent)]
    Gauges(#[from] GaugeError),
    #[error(transparent)]
    Settlement(#[from] SettlementError),
    #[error(transparent)]
    Reply(#[from] CounterReplyError),
    #[error(transparent)]
    Counter(#[from] CounterValueError),
    #[error("the pipelined rate limiter batch failed: {0}")]
    Batch(Arc<BatchError>),
    #[error(
        "the windowed check got {keys} keys and {values} values, which are not window/counter pairs"
    )]
    WindowPairCount { keys: usize, values: usize },
    #[error("no window metadata was collected for the window key {window_key}")]
    MissingWindowMetadata { window_key: String },
}
