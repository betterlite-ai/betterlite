use std::time::Instant;

use super::RedisCacheValues;
use super::error::RedisCacheValuesError;
use crate::redis_client_reuse::breaker::BreakerAdmission;
use crate::redis_client_reuse::service_event::ServiceEvent;

pub(crate) fn event(
    call_type: &'static str,
    started: Instant,
    key: Option<String>,
) -> ServiceEvent {
    ServiceEvent {
        call_type,
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
