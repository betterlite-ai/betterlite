use betterlite_models::decisions::Json;
use betterlite_models::metadata::levels::ReasoningEffort;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use betterlite_openai::openai_base_config::{ModelFamily, azure_model_family};

use self::declared::{
    ADVERTISEMENT_ORDER, EffortFlag, EffortFlags, declared_effort_flags,
    declared_reasoning_efforts, text,
};
use crate::router_predicates::JsonMap;

pub mod declared;

// REASONING_EFFORT_STRENGTH_ORDER: none is the off switch and never sits on this ladder.
pub const STRENGTH_ORDER: [ReasoningEffort; 6] = [
    ReasoningEffort::Minimal,
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
    ReasoningEffort::Xhigh,
    ReasoningEffort::Max,
];

const AZURE: &str = "azure";

// nearest_declared_reasoning_effort: round up to the weakest declared level that meets the request, and down to the strongest one above every level.
#[must_use]
pub fn nearest_declared_effort(
    requested: ReasoningEffort,
    declared: &[ReasoningEffort],
) -> ReasoningEffort {
    let Some(requested_rank) = STRENGTH_ORDER.iter().position(|level| *level == requested) else {
        return requested;
    };
    let mut ranked: Vec<ReasoningEffort> = declared
        .iter()
        .copied()
        .filter(|effort| STRENGTH_ORDER.contains(effort))
        .collect();
    ranked.sort_by_key(|effort| STRENGTH_ORDER.iter().position(|level| level == effort));
    if ranked.contains(&requested) || ranked.is_empty() {
        return requested;
    }
    ranked
        .iter()
        .copied()
        .find(|effort| {
            STRENGTH_ORDER
                .iter()
                .position(|level| level == effort)
                .is_some_and(|rank| rank >= requested_rank)
        })
        .or_else(|| ranked.last().copied())
        .unwrap_or(requested)
}

// deployment_is_catalog_mapped: the map described the deployment, the operator did not.
#[must_use]
pub fn deployment_is_catalog_mapped(
    resolved_model_info: Option<&JsonMap>,
    operator_model_info: &JsonMap,
) -> bool {
    let Some(resolved) = resolved_model_info else {
        return false;
    };
    resolved
        .get("mode")
        .is_some_and(|value| !matches!(value, Json::Null))
        && operator_model_info
            .get("mode")
            .is_none_or(|value| matches!(value, Json::Null))
}

// resolve_supported_reasoning_efforts: None is unknown, a Some of an empty list is a known model that accepts no level.
#[must_use]
pub fn resolve_supported_efforts(
    model_info: &JsonMap,
    deployment_is_mapped: bool,
    model_cost: &ModelCostMap,
) -> Option<Vec<ReasoningEffort>> {
    let supports_reasoning = model_info
        .get("supports_reasoning")
        .filter(|value| !matches!(value, Json::Null));
    if matches!(supports_reasoning, Some(Json::Bool(false))) {
        return Some(Vec::new());
    }
    let flags = declared_effort_flags(model_info, model_cost);
    if !matches!(supports_reasoning, Some(Json::Bool(true))) && !flags.any_present() {
        return deployment_is_mapped.then(Vec::new);
    }
    if let Some(declared) = declared_reasoning_efforts(model_info, model_cost) {
        return Some(declared);
    }
    if flags.all_absent() {
        return None;
    }
    Some(advertised_levels(model_info, flags))
}

// The opt-out, unconditional and opt-in levels a reasoning model advertises from its flags.
fn advertised_levels(model_info: &JsonMap, flags: EffortFlags) -> Vec<ReasoningEffort> {
    let mut allowed: Vec<ReasoningEffort> = Vec::new();
    for effort in [ReasoningEffort::Minimal, ReasoningEffort::Low] {
        if !matches!(flags.get(effort), EffortFlag::False) {
            allowed.push(effort);
        }
    }
    allowed.push(ReasoningEffort::Medium);
    allowed.push(ReasoningEffort::High);
    for effort in [ReasoningEffort::Xhigh, ReasoningEffort::Max] {
        if matches!(flags.get(effort), EffortFlag::True) {
            allowed.push(effort);
        }
    }
    if supports_none_reasoning_effort(model_info, flags.none) {
        allowed.push(ReasoningEffort::None);
    }
    ADVERTISEMENT_ORDER
        .into_iter()
        .filter(|effort| allowed.contains(effort))
        .collect()
}

// _supports_none_reasoning_effort: azure gpt-5 requires an explicit true, everything else opts out.
#[must_use]
pub fn supports_none_reasoning_effort(model_info: &JsonMap, flag: EffortFlag) -> bool {
    if text(model_info, "litellm_provider") != Some(AZURE) {
        return !matches!(flag, EffortFlag::False);
    }
    let Some(key) = text(model_info, "key") else {
        return !matches!(flag, EffortFlag::False);
    };
    if azure_model_family(key) != ModelFamily::GptReasoning {
        return !matches!(flag, EffortFlag::False);
    }
    matches!(flag, EffortFlag::True)
}

// intersect_supported_efforts: None never narrows, and an effort survives every deployment.
#[must_use]
pub fn intersect_supported_efforts(
    current: Option<&[ReasoningEffort]>,
    resolved: Option<&[ReasoningEffort]>,
) -> Option<Vec<ReasoningEffort>> {
    match (current, resolved) {
        (_, None) => current.map(<[ReasoningEffort]>::to_vec),
        (None, Some(found)) => Some(found.to_vec()),
        (Some(held), Some(found)) => Some(
            ADVERTISEMENT_ORDER
                .into_iter()
                .filter(|effort| held.contains(effort) && found.contains(effort))
                .collect(),
        ),
    }
}

// The model-group aggregate get_model_group_info builds out of each deployment's answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupReasoningEfforts {
    pub(super) supported: Option<Vec<ReasoningEffort>>,
    pub(super) unknown: bool,
}

impl GroupReasoningEfforts {
    pub fn observe(&mut self, deployment: Option<Vec<ReasoningEffort>>) {
        match deployment {
            None => {
                self.unknown = true;
                self.supported = None;
            }
            Some(levels) if !self.unknown => {
                self.supported =
                    intersect_supported_efforts(self.supported.as_deref(), Some(&levels));
            }
            Some(_) => {}
        }
    }
}
