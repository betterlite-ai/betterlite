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

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::atomic_counter::payload::IncrementAmounts;
use crate::atomic_counter::response::ResponseDecodeError;
use crate::atomic_counter::value::CounterValueError as AtomicCounterValueError;
use crate::fail_closed::{FailClosedPolicy, RateLimitUnverifiableError};
use crate::limiter_records::{
    CounterRefund, CounterReplyError, LimiterClock, RateLimitDescriptor, RateLimitResponse,
};
use crate::rate_limit_window_store::counter_value::CounterValueError;
use crate::rate_limit_window_store::{CounterScript, WindowStore};

pub struct AtomicSettlement<'settlement> {
    pub windows: &'settlement WindowStore,
    pub clock: &'settlement dyn LimiterClock,
    pub script: Option<Arc<dyn CounterScript>>,
    pub refund_script: Option<Arc<dyn CounterScript>>,
    pub fail_closed: FailClosedPolicy,
    pub default_window_size: i64,
}

impl AtomicSettlement<'_> {
    // ast-grep-ignore: no-long-names
    pub async fn atomic_check_and_increment_by_n(
        &self,
        descriptors: &[RateLimitDescriptor],
        increments: &[IncrementAmounts],
    ) -> Result<RateLimitResponse, SettlementError> {
        let groups =
            entry_point::descriptor_groups(descriptors, increments, self.default_window_size)?;
        if groups.is_empty() {
            return Ok(RateLimitResponse {
                overall_code: "OK".to_owned(),
                statuses: Vec::new(),
                reservation_windows: None,
            });
        }
        dispatch::dispatch(self, &groups).await
    }

    pub async fn refund_counter_increments(&self, refunds: &[CounterRefund]) {
        let Some(script) = self.refund_script.as_deref() else {
            return;
        };
        for refund in refunds {
            let keys = vec![refund.window_key.clone(), refund.counter_key.clone()];
            let args = vec![
                RedisScalar::Text(refund.window_start.clone()),
                RedisScalar::Text(i128::from(refund.increment).saturating_neg().to_string()),
                RedisScalar::Text("0".to_owned()),
            ];
            let Err(error) = script.run(&keys, &args).await else {
                continue;
            };
            tracing::warn!(
                %error,
                counter_key = %refund.counter_key,
                "failed to refund a counter on rollback"
            );
        }
    }

    // ast-grep-ignore: no-long-names
    pub(crate) async fn refund_applied_descriptor_groups(&self, applied: &[Vec<CounterRefund>]) {
        let mut flattened: Vec<CounterRefund> = Vec::new();
        for group in applied {
            flattened.extend(group.iter().cloned());
        }
        self.refund_counter_increments(&flattened).await;
    }
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
