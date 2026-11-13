use std::error::Error;
use std::time::Instant;

use super::RedisCacheValues;
use super::error::RedisCacheValuesError;
use crate::cache_primitives::decode::{CachedValue, decode_cached_value};
use crate::redis_client_reuse::breaker::BreakerAdmission;
use crate::redis_client_reuse::error::ClientReuseError;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_live_client::{LiveAsyncClient, LiveClientError};

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
    read: Result<Option<Vec<u8>>, RedisCacheValuesError>,
) -> Option<CachedValue> {
    let bytes = match read {
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

pub(crate) async fn open_client(
    values: &RedisCacheValues,
) -> Result<LiveAsyncClient, RedisCacheValuesError> {
    match values.transport.async_client().await {
        Ok(client) => Ok(client),
        Err(source) => Err(RedisCacheValuesError::ClientReuse(source)),
    }
}

// litellm's init_async_client only builds an unconnected client, so a connect error belongs to the command phase
pub(crate) const fn connection_failure(error: &RedisCacheValuesError) -> bool {
    matches!(
        error,
        RedisCacheValuesError::ClientReuse(ClientReuseError::Live(
            LiveClientError::Redis(_)
                | LiveClientError::NoSentinelMaster { .. }
                | LiveClientError::TlsFile { .. }
        ))
    )
}

pub(crate) async fn client_or_swallowed(
    values: &RedisCacheValues,
    name: &'static str,
    started: Instant,
    admission: BreakerAdmission,
    key: Option<String>,
) -> Result<Option<LiveAsyncClient>, RedisCacheValuesError> {
    match open_client(values).await {
        Ok(client) => Ok(Some(client)),
        Err(error) => {
            let event_key = connection_failure(&error).then_some(key).flatten();
            let failed = event(name, started, event_key);
            values.hooks.failure(&failed, &error);
            if connection_failure(&error) {
                values.gate.swallowed(&admission, &error);
                values.gate.exit(admission);
                return Ok(None);
            }
            values.gate.record_failure(&error);
            Err(error)
        }
    }
}
