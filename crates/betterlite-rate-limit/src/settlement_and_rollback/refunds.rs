use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::limiter_records::{AtomicCounterMeta, CacheCounterValue, CounterRefund};
use crate::rate_limit_window_store::CounterValues;
use crate::rate_limit_window_store::counter_value::counter_as_int;

use super::{AtomicSettlement, SettlementError};

#[must_use]
// ast-grep-ignore: no-long-names
pub fn counter_refunds_from_atomic_response(
    raw: &[CacheCounterValue],
    per_counter_meta: &[AtomicCounterMeta],
) -> Result<Vec<CounterRefund>, SettlementError> {
    let count = raw.len();
    let mut refunds = Vec::with_capacity(per_counter_meta.len());
    for (index, meta) in per_counter_meta.iter().enumerate() {
        let position = index.saturating_mul(2).saturating_add(2);
        let value = raw
            .get(position)
            .ok_or(SettlementError::RefundPosition { position, count })?;
        refunds.push(CounterRefund {
            window_key: meta.window_key.clone(),
            counter_key: meta.counter_key.clone(),
            window_start: counter_as_int(value)?.to_string(),
            increment: meta.increment,
        });
    }
    Ok(refunds)
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn counter_refunds_from_batch_values(
    applied_keys: &[String],
    applied_values: &CounterValues,
) -> Result<Vec<CounterRefund>, SettlementError> {
    let pairs = applied_keys
        .len()
        .div_ceil(2)
        .min(applied_values.len().div_ceil(2));
    let mut refunds = Vec::with_capacity(pairs);
    for pair in 0..pairs {
        let offset = pair.saturating_mul(2);
        let Some(window_start) = applied_values.get(offset).and_then(Option::as_ref) else {
            continue;
        };
        let window_key = applied_keys
            .get(offset)
            .ok_or(SettlementError::UnpairedRefundKeys {
                count: applied_keys.len(),
            })?;
        let counter_key = applied_keys.get(offset.saturating_add(1)).ok_or(
            SettlementError::UnpairedRefundKeys {
                count: applied_keys.len(),
            },
        )?;
        refunds.push(CounterRefund {
            window_key: window_key.clone(),
            counter_key: counter_key.clone(),
            window_start: counter_as_int(window_start)?.to_string(),
            increment: 1,
        });
    }
    Ok(refunds)
}

impl AtomicSettlement<'_> {
    // ast-grep-ignore: no-long-names
    pub(super) async fn refund_applied_descriptor_groups(&self, applied: &[Vec<CounterRefund>]) {
        let mut flattened: Vec<CounterRefund> = Vec::new();
        for group in applied {
            flattened.extend(group.iter().cloned());
        }
        self.refund_counter_increments(&flattened).await;
    }

    pub async fn refund_counter_increments(&self, refunds: &[CounterRefund]) {
        let Some(script) = self.refund_script.as_deref() else {
            return;
        };
        for refund in refunds {
            let keys = vec![refund.window_key.clone(), refund.counter_key.clone()];
            let args = vec![
                RedisScalar::Text(refund.window_start.clone()),
                RedisScalar::Text((-i128::from(refund.increment)).to_string()),
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
}
