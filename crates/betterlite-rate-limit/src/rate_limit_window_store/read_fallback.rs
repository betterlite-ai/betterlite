use super::batch_get::batch_get_counter_values;
use super::redis_read_order::read_counter_values_from_redis;
use super::{CounterValues, WindowStore};
use crate::fail_closed::{FailClosedPolicy, RateLimitUnverifiableError};

pub async fn read_counter_snapshot(
    store: &WindowStore,
    policy: &FailClosedPolicy,
    keys: &[String],
) -> Result<Option<CounterValues>, RateLimitUnverifiableError> {
    if store.counter_read_script.is_none() {
        return Ok(batch_get_counter_values(store, keys, false).await);
    }
    match read_counter_values_from_redis(store, keys).await {
        Ok(values) => Ok(Some(values)),
        Err(error) => {
            policy.reject_if_unverifiable("batch_counter_read_script", &error)?;
            tracing::warn!(error = %error, "batch_counter_read_script failed, using local mirror");
            Ok(batch_get_counter_values(store, keys, true).await)
        }
    }
}

mod tests;
