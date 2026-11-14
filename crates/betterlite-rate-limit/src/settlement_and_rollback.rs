pub mod dispatch;
pub mod entry_point;
pub mod group_response;
pub mod lua_per_descriptor;
pub mod pipelined_settlement;
pub mod refunds;
pub mod sequential_loop;
pub mod single_descriptor;

mod batch_refund;
mod pipelined_aggregate;
mod pipelined_exception;
mod pipelined_gather;
mod pipelined_over_limit;
mod refund_extraction;
mod refund_failure;
mod refund_script;
mod rollback_exception;
mod rollback_over_limit;

mod tests;

use std::num::TryFromIntError;
use std::sync::Arc;
use std::time::SystemTimeError;

use crate::atomic_counter::response::ResponseDecodeError;
use crate::atomic_counter::value::CounterValueError as AtomicCounterValueError;
use crate::fail_closed::{FailClosedPolicy, RateLimitUnverifiableError};
use crate::limiter_records::{CounterReplyError, LimiterClock};
use crate::rate_limit_window_store::counter_value::CounterValueError;
use crate::rate_limit_window_store::{CounterScript, WindowStore};

pub struct AtomicSettlement<'a> {
    pub windows: &'a WindowStore,
    pub clock: &'a dyn LimiterClock,
    pub script: Option<Arc<dyn CounterScript>>,
    pub refund_script: Option<Arc<dyn CounterScript>>,
    pub fail_closed: FailClosedPolicy,
    pub default_window_size: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum SettlementError {
    #[error(
        "the atomic check got {descriptors} descriptors and {increments} increment amounts, which differ"
    )]
    DescriptorCount {
        descriptors: usize,
        increments: usize,
    },
    #[error(transparent)]
    Reply(#[from] CounterReplyError),
    #[error(transparent)]
    Response(#[from] ResponseDecodeError),
    #[error(transparent)]
    AtomicCounter(#[from] AtomicCounterValueError),
    #[error(transparent)]
    WindowCounter(#[from] CounterValueError),
    #[error("the settlement clock reads a time before the Unix epoch")]
    ClockBeforeEpoch(#[source] SystemTimeError),
    #[error("the settlement clock reads {value} seconds, which has no signed 64-bit count")]
    ClockOverflow {
        value: u64,
        #[source]
        source: TryFromIntError,
    },
    #[error("the batch refund keys are not window/counter pairs: {count} keys")]
    UnpairedRefundKeys { count: usize },
    #[error("the Lua reply holds {count} values, so it has no window start at position {position}")]
    RefundPosition { position: usize, count: usize },
    #[error(transparent)]
    Unverifiable(#[from] RateLimitUnverifiableError),
}
