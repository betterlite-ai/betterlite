use std::error::Error;
use std::time::Instant;

use super::RedisCacheValues;
use super::error::RedisCacheValuesError;
use crate::cache_primitives::decode::{CachedValue, decode_cached_value};
use crate::redis_client_reuse::breaker::BreakerAdmission;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_live_client::LiveAsyncClient;

pub(crate) fn event(kind: &'static str, started: Instant, key: Option<String>) -> ServiceEvent {
    ServiceEvent {
        call_type: kind,
        duration: started.elapsed(),
        key,
    }
}

pub(crate) fn finish_outcome<T>(
    values: &RedisCacheValues,
    event: &ServiceEvent,
    admission: BreakerAdmission,
    outcome: Result<T, RedisCacheValuesError>,
) -> Option<T> {
    match outcome {
        Ok(value) => {
            values.hooks.success(event);
            values.gate.exit(admission);
            Some(value)
        }
        Err(error) => {
            values.hooks.failure(event, &error);
            values.gate.swallowed(&admission, &error);
            values.gate.exit(admission);
            None
        }
    }
}

pub(crate) fn finish_get(
    values: &RedisCacheValues,
    event: &ServiceEvent,
    admission: BreakerAdmission,
    reply: Result<Option<Vec<u8>>, RedisCacheValuesError>,
) -> Option<CachedValue> {
    let bytes = match reply {
        Ok(bytes) => bytes,
        Err(error) => {
            swallow_read(values, admission, &error);
            return None;
        }
    };
    values.hooks.success(event);
    decode_or_swallow(values, admission, bytes.as_deref())
}

fn decode_or_swallow(
    values: &RedisCacheValues,
    admission: BreakerAdmission,
    bytes: Option<&[u8]>,
) -> Option<CachedValue> {
    match decode_cached_value(bytes) {
        Ok(value) => {
            values.gate.exit(admission);
            value
        }
        Err(error) => {
            swallow_read(values, admission, &error);
            None
        }
    }
}

pub(crate) fn swallow_read<E>(values: &RedisCacheValues, admission: BreakerAdmission, error: &E)
where
    E: Error + Send + Sync,
{
    values.gate.swallowed(&admission, error);
    values.gate.exit(admission);
}

pub(crate) fn enter_unless_empty(
    values: &RedisCacheValues,
    name: &'static str,
    empty: bool,
) -> Result<Option<BreakerAdmission>, RedisCacheValuesError> {
    let admission = values.gate.enter(name)?;
    if empty {
        values.gate.exit(admission);
        return Ok(None);
    }
    Ok(Some(admission))
}

pub(crate) async fn recorded_client(
    values: &RedisCacheValues,
) -> Result<LiveAsyncClient, RedisCacheValuesError> {
    let opened = values.transport.async_client().await;
    opened.map_err(|source| {
        values.gate.record_failure(&source);
        RedisCacheValuesError::ClientReuse(source)
    })
}

pub(crate) async fn async_client_or_fail(
    values: &RedisCacheValues,
    name: &'static str,
    started: Instant,
) -> Result<LiveAsyncClient, RedisCacheValuesError> {
    match recorded_client(values).await {
        Ok(client) => Ok(client),
        Err(error) => {
            let failed = event(name, started, None);
            values.hooks.failure(&failed, &error);
            Err(error)
        }
    }
}
