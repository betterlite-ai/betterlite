use std::collections::BTreeMap;

use betterlite_models::model_registration::JsonObject;

use crate::health_cycle::ports::ProbeFailure;

// _write_health_state_to_router_cache as the loop reaches it: B8388 owns what the write does with the rows.
pub trait HealthStateWriter: Send + Sync {
    fn write(
        &self,
        healthy: &[JsonObject],
        unhealthy: &[JsonObject],
        exceptions: &BTreeMap<String, ProbeFailure>,
    );
}
