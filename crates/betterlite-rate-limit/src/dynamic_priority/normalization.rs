use std::collections::BTreeMap;
use std::ops::Div as _;

use crate::provider_attribution::{ModelGroupLimits, PriorityValue, convert_priority_to_percent};

use super::settings::PriorityPolicy;

mod tests;

// _normalize_priority_weights: shares that add up past the model's capacity are scaled back so each keeps its proportion
#[must_use]
pub fn normalize_priority_weights(
    policy: &PriorityPolicy,
    model_info: Option<&ModelGroupLimits>,
) -> BTreeMap<String, f64> {
    let Some(reservation) = &policy.reservation else {
        return BTreeMap::new();
    };
    let weights = converted_weights(reservation, model_info);
    let total = weights.values().sum::<f64>();
    if total > 1.0_f64 {
        let normalized = scaled_weights(&weights, total);
        tracing::debug!("Normalized over-allocated priorities: {weights:?} -> {normalized:?}");
        return normalized;
    }
    weights
}

fn converted_weights(
    reservation: &BTreeMap<String, PriorityValue>,
    model_info: Option<&ModelGroupLimits>,
) -> BTreeMap<String, f64> {
    let mut weights = BTreeMap::new();
    for (priority, value) in reservation {
        weights.insert(
            priority.clone(),
            convert_priority_to_percent(*value, model_info),
        );
    }
    weights
}

fn scaled_weights(weights: &BTreeMap<String, f64>, total: f64) -> BTreeMap<String, f64> {
    let mut normalized = BTreeMap::new();
    for (priority, weight) in weights {
        normalized.insert(priority.clone(), weight.div(total));
    }
    normalized
}
