use betterlite_models::metadata::levels::ReasoningEffort;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use betterlite_router::routing_groups::effort::declared::{
    declared_efforts_for_model, effort_from_wire, effort_wire_name,
};
use betterlite_router::routing_groups::effort::{
    deployment_is_catalog_mapped, intersect_supported_efforts, nearest_declared_effort,
    resolve_supported_efforts,
};

use crate::router_probes::error::OperationError;

use super::{EffortInput, Input, IntersectionInput, ModelLookup, Output, RoundInput};

fn wire_names(levels: &[ReasoningEffort]) -> Vec<String> {
    levels
        .iter()
        .map(|level| effort_wire_name(*level).to_owned())
        .collect()
}

fn effort(name: &str) -> Result<ReasoningEffort, OperationError> {
    match effort_from_wire(name) {
        Some(level) => Ok(level),
        None => Err(OperationError::InvalidRequest {
            message: format!("unknown reasoning effort {name}"),
        }),
    }
}

fn parsed_levels(names: &[String]) -> Result<Vec<ReasoningEffort>, OperationError> {
    let mut levels = Vec::with_capacity(names.len());
    for name in names {
        levels.push(effort(name)?);
    }
    Ok(levels)
}

fn effort_result(case: &EffortInput, cost_map: &ModelCostMap) -> Option<Vec<String>> {
    let mapped = deployment_is_catalog_mapped(case.resolved_model_info.as_ref(), &case.model_info);
    let resolved = resolve_supported_efforts(&case.model_info, mapped, cost_map);
    resolved.map(|levels| wire_names(&levels))
}

fn intersection_result(case: &IntersectionInput) -> Result<Option<Vec<String>>, OperationError> {
    let current = case.current.as_deref().map(parsed_levels).transpose()?;
    let resolved = case.resolved.as_deref().map(parsed_levels).transpose()?;
    Ok(
        intersect_supported_efforts(current.as_deref(), resolved.as_deref())
            .map(|levels| wire_names(&levels)),
    )
}

fn round_result(case: &RoundInput) -> Result<String, OperationError> {
    let requested = effort(&case.requested)?;
    let declared = parsed_levels(&case.declared)?;
    Ok(effort_wire_name(nearest_declared_effort(requested, &declared)).to_owned())
}

fn declared_result(lookup: &ModelLookup, cost_map: &ModelCostMap) -> Option<Vec<String>> {
    declared_efforts_for_model(&lookup.model, &lookup.custom_llm_provider, cost_map)
        .map(|levels| wire_names(&levels))
}

pub(super) fn compute(input: &Input, cost_map: &ModelCostMap) -> Result<Output, OperationError> {
    let efforts: Vec<Option<Vec<String>>> = input
        .efforts
        .iter()
        .map(|case| effort_result(case, cost_map))
        .collect();
    let intersections = input
        .intersections
        .iter()
        .map(intersection_result)
        .collect::<Result<Vec<_>, _>>()?;
    let rounds = input
        .rounds
        .iter()
        .map(round_result)
        .collect::<Result<Vec<_>, _>>()?;
    let declared: Vec<Option<Vec<String>>> = input
        .lookups
        .iter()
        .map(|lookup| declared_result(lookup, cost_map))
        .collect();
    Ok(Output {
        groups: Vec::new(),
        error: None,
        names: Vec::new(),
        efforts,
        intersections,
        rounds,
        declared,
    })
}
