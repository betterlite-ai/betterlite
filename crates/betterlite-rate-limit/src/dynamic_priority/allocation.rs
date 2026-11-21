use std::collections::BTreeMap;

use crate::provider_attribution::ModelGroupLimits;

use super::settings::PriorityPolicy;
use super::weight::priority_weight;

mod tests;

// _get_priority_allocation's result: the share this request may use and the counter pool it shares with its peers
#[derive(Clone, Debug, PartialEq)]
pub struct PriorityAllocation {
    pub weight: f64,
    pub key: String,
}

// _get_priority_allocation: an explicit priority gets its own pool, and every request without one shares the default pool
#[must_use]
pub fn priority_allocation(
    policy: &PriorityPolicy,
    model: &str,
    priority: Option<&str>,
    normalized_weights: &BTreeMap<String, f64>,
    model_info: Option<&ModelGroupLimits>,
) -> PriorityAllocation {
    let Some(chosen) = priority.filter(|name| is_reserved(policy, name)) else {
        return PriorityAllocation {
            weight: policy.settings.default_priority,
            key: format!("{model}:default_pool"),
        };
    };
    PriorityAllocation {
        weight: normalized_weights
            .get(chosen)
            .copied()
            .unwrap_or_else(|| priority_weight(policy, Some(chosen), model_info)),
        key: format!("{model}:{chosen}"),
    }
}

// has_explicit_priority: the reservation is the only place a priority can come from
fn is_reserved(policy: &PriorityPolicy, name: &str) -> bool {
    let Some(reservation) = &policy.reservation else {
        return false;
    };
    reservation.contains_key(name)
}
