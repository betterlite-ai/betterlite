use betterlite_models::decisions::Json;
use betterlite_models::metadata::levels::ReasoningEffort;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use betterlite_openai::openai_base_config::{ModelFamily, azure_model_family};

use self::declared::{
    ADVERTISEMENT_ORDER, EffortFlag, EffortFlags, declared_effort_flags,
    declared_reasoning_efforts, field, text,
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

// nearest_declared_reasoning_effort: round up to the weakest declared level that still meets the
// request, and down to the strongest one when it asks for more than the model has.
#[must_use]
pub fn nearest_declared_reasoning_effort(
    requested: ReasoningEffort,
    declared: &[ReasoningEffort],
) -> ReasoningEffort {
    let Some(requested_rank) = strength_rank(requested) else {
        return requested;
    };
    let mut ranked: Vec<ReasoningEffort> = declared
        .iter()
        .copied()
        .filter(|effort| strength_rank(*effort).is_some())
        .collect();
    ranked.sort_by_key(|effort| strength_rank(*effort));
    if ranked.contains(&requested) || ranked.is_empty() {
        return requested;
    }
    ranked
        .iter()
        .copied()
        .find(|effort| strength_rank(*effort).is_some_and(|rank| rank >= requested_rank))
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
    field(resolved, "mode").is_some() && field(operator_model_info, "mode").is_none()
}

// resolve_supported_reasoning_efforts: None is unknown, Some of an empty list a known model that
// accepts no effort level.
#[must_use]
pub fn resolve_supported_reasoning_efforts(
    model_info: &JsonMap,
    deployment_is_mapped: bool,
    model_cost: &ModelCostMap,
) -> Option<Vec<ReasoningEffort>> {
    let supports_reasoning = field(model_info, "supports_reasoning");
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
    Some(advertised_levels(model_info, &flags))
}

// The opt-out, unconditional and opt-in levels a reasoning model advertises from its flags.
fn advertised_levels(model_info: &JsonMap, flags: &EffortFlags) -> Vec<ReasoningEffort> {
    let mut allowed: Vec<ReasoningEffort> = Vec::new();
    for effort in [ReasoningEffort::Minimal, ReasoningEffort::Low] {
        if !flags.get(effort).is_false() {
            allowed.push(effort);
        }
    }
    allowed.push(ReasoningEffort::Medium);
    allowed.push(ReasoningEffort::High);
    for effort in [ReasoningEffort::Xhigh, ReasoningEffort::Max] {
        if flags.get(effort).is_true() {
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
        return !flag.is_false();
    }
    let Some(key) = text(model_info, "key") else {
        return !flag.is_false();
    };
    if azure_model_family(key) != ModelFamily::GptReasoning {
        return !flag.is_false();
    }
    flag.is_true()
}

// intersect_supported_reasoning_efforts: None never narrows, and an effort survives every deployment.
#[must_use]
pub fn intersect_supported_reasoning_efforts(
    current: Option<&[ReasoningEffort]>,
    resolved: Option<&[ReasoningEffort]>,
) -> Option<Vec<ReasoningEffort>> {
    match (current, resolved) {
        (_, None) => current.map(<[ReasoningEffort]>::to_vec),
        (None, Some(resolved)) => Some(resolved.to_vec()),
        (Some(current), Some(resolved)) => Some(
            ADVERTISEMENT_ORDER
                .into_iter()
                .filter(|effort| current.contains(effort) && resolved.contains(effort))
                .collect(),
        ),
    }
}

// The model-group aggregate get_model_group_info builds out of each deployment's answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupReasoningEfforts {
    supported: Option<Vec<ReasoningEffort>>,
    unknown: bool,
    initialized: bool,
}

impl GroupReasoningEfforts {
    pub fn observe(&mut self, deployment: Option<Vec<ReasoningEffort>>) {
        match deployment {
            None => {
                self.unknown = true;
                self.supported = None;
            }
            Some(levels) if !self.initialized => {
                self.initialized = true;
                if !self.unknown {
                    self.supported = Some(levels);
                }
            }
            Some(levels) if !self.unknown => {
                self.supported =
                    intersect_supported_reasoning_efforts(self.supported.as_deref(), Some(&levels));
            }
            Some(_) => {}
        }
    }

    #[must_use]
    pub fn supported(&self) -> Option<&[ReasoningEffort]> {
        self.supported.as_deref()
    }

    #[must_use]
    pub const fn is_unknown(&self) -> bool {
        self.unknown
    }
}

fn strength_rank(effort: ReasoningEffort) -> Option<usize> {
    STRENGTH_ORDER.iter().position(|level| *level == effort)
}
