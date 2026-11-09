use std::time::Instant;

use crate::redis_client_reuse::breaker::BreakerAdmission;
use crate::redis_client_reuse::service_event::ServiceEvent;

use super::RedisListOperations;
use super::error::RedisListOperationsError;

pub(crate) struct Call {
    pub(crate) name: &'static str,
    pub(crate) key: Option<String>,
    pub(crate) log: &'static str,
}

// litellm raises from the guarded method, so a failed call records the failure and never reaches exit
pub(crate) fn finish<T>(
    lists: &RedisListOperations,
    admission: BreakerAdmission,
    call: Call,
    started: Instant,
    outcome: Result<T, RedisListOperationsError>,
) -> Result<T, RedisListOperationsError> {
    let event = ServiceEvent {
        call_type: call.name,
        duration: started.elapsed(),
        key: call.key,
    };
    match outcome {
        Ok(value) => {
            lists.hooks.success(&event);
            lists.gate.exit(admission);
            Ok(value)
        }
        Err(error) => {
            lists.hooks.failure(&event, &error);
            tracing::error!(error = %error, "{}", call.log);
            lists.gate.record_failure(&error);
            Err(error)
        }
    }
}
