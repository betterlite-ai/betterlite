use crate::redis_client_reuse::breaker::BreakerAdmission;
use crate::redis_client_reuse::breaker_gate::BreakerGate;

use super::RedisCountersError;

pub(crate) fn finish_guard<T>(
    gate: &dyn BreakerGate,
    admission: BreakerAdmission,
    outcome: Result<T, RedisCountersError>,
) -> Result<T, RedisCountersError> {
    match outcome {
        Ok(value) => {
            gate.exit(admission);
            Ok(value)
        }
        Err(error) => {
            gate.record_failure(&error);
            Err(error)
        }
    }
}
