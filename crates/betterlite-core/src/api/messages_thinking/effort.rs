use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::metadata::levels::ReasoningEffort;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use betterlite_openai::reasoning_effort_capability::declared::effort_wire_name;
use betterlite_openai::reasoning_effort_capability::resolve_supported_efforts;

// _EFFORT_DEGRADATION_CHAIN, with the floor a deployment that accepts no chain member keeps.
fn degradation(effort: &str) -> Option<(&'static [&'static str], &'static str)> {
    match effort {
        "max" => Some((&["max", "xhigh", "high"], "high")),
        "xhigh" => Some((&["xhigh", "high"], "high")),
        "minimal" => Some((&["minimal", "low"], "low")),
        _ => None,
    }
}

// A tier the deployment refuses drops to the nearest chain member it accepts; none is the off switch, never a tier.
#[must_use]
pub fn degrade_effort<'effort>(
    effort: &'effort str,
    supported: Option<&[ReasoningEffort]>,
) -> &'effort str {
    let Some((chain, floor)) = degradation(effort) else {
        return effort;
    };
    let Some(accepted) = supported.filter(|levels| !levels.is_empty()) else {
        return floor;
    };
    if let Some(level) = chain.iter().find(|level| {
        accepted
            .iter()
            .any(|supported| effort_wire_name(*supported) == **level)
    }) {
        return *level;
    }
    accepted
        .iter()
        .find(|supported| **supported != ReasoningEffort::None)
        .map_or(floor, |supported| effort_wire_name(*supported))
}

// normalize_reasoning_effort_value: get_model_info raising for an unmapped model leaves no metadata, which keeps the floor.
#[must_use]
pub fn normalize_reasoning_effort<'effort>(
    effort: &'effort str,
    model_info: Option<&ModelInfo>,
    model_cost: &ModelCostMap,
) -> &'effort str {
    let supported =
        model_info.and_then(|info| resolve_supported_efforts(&info.fields, true, model_cost));
    degrade_effort(effort, supported.as_deref())
}
