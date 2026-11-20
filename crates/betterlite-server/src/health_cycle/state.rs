use betterlite_models::model_registration::JsonObject;
use betterlite_router::cooldown_state::health::{HealthSnapshot, HealthState};
use betterlite_router::routing_value_store::Clock;

use crate::health_cycle::endpoints::endpoint_id;

const FAILURE_REASON: &str = "background_health_check_failed";

// build_deployment_health_states: healthy rows first, then unhealthy rows overwrite a shared id with the failure verdict
#[must_use]
pub fn build_deployment_health_states(
    healthy_endpoints: &[JsonObject],
    unhealthy_endpoints: &[JsonObject],
    clock: &Clock,
) -> HealthSnapshot {
    let now = (clock)();
    let mut states = HealthSnapshot::new();
    for endpoint in healthy_endpoints {
        if let Some(model_id) = endpoint_id(endpoint) {
            states.insert(
                model_id.to_owned(),
                HealthState {
                    is_healthy: true,
                    timestamp: now,
                    reason: String::new(),
                },
            );
        }
    }
    for endpoint in unhealthy_endpoints {
        if let Some(model_id) = endpoint_id(endpoint) {
            states.insert(
                model_id.to_owned(),
                HealthState {
                    is_healthy: false,
                    timestamp: now,
                    reason: FAILURE_REASON.to_owned(),
                },
            );
        }
    }
    states
}
