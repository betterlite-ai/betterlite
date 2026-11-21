use crate::provider_attribution::{ModelGroupLimits, convert_priority_to_percent};

use super::settings::PriorityPolicy;

mod tests;

// _get_priority_weight: a priority the reservation does not name keeps the policy's default share of capacity
#[must_use]
pub fn priority_weight(
    policy: &PriorityPolicy,
    priority: Option<&str>,
    model_info: Option<&ModelGroupLimits>,
) -> f64 {
    let Some(reservation) = &policy.reservation else {
        tracing::debug!("Priority Reservation not set for the given priority.");
        return policy.settings.default_priority;
    };
    let Some(value) = priority.and_then(|name| reservation.get(name)) else {
        tracing::debug!("Priority Reservation not set for the given priority.");
        return policy.settings.default_priority;
    };
    convert_priority_to_percent(*value, model_info)
}
