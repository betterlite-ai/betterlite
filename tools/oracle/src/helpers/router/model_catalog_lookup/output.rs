use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::listing::should_include_deployment;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use serde_json::{Value, json};

use super::error::Failure;
use super::input::{Arguments, Function, Input};
use super::snapshot::Snapshot;

// The answer the case asked for: the production lookup or listing call, projected to the fields LiteLLM's rows expose.
pub(crate) fn answer(input: &Input) -> Result<Value, Failure> {
    let snapshot = Snapshot::build(input)?;
    let lookup = CatalogLookup {
        catalog: &snapshot.catalog,
        model_group_alias: &snapshot.aliases,
        routing_groups: &snapshot.groups,
        inferred_provider: &*snapshot.provider,
    };
    call_answer(&lookup, &snapshot, input)
}

// The call the case named, each answer read from the argument it needs.
fn call_answer(
    lookup: &CatalogLookup<'_>,
    snapshot: &Snapshot,
    input: &Input,
) -> Result<Value, Failure> {
    let arguments = &input.arguments;
    let name = arguments.model_name.as_deref();
    let team = arguments.team_id.as_deref();
    match input.function {
        Function::GetDeployment => {
            let id = arguments
                .model_id
                .as_deref()
                .ok_or(Failure::Argument("model_id"))?;
            let row = lookup.get_deployment(id)?;
            Ok(row.map_or(Value::Null, deployment_row))
        }
        Function::GetDeploymentByModelGroupName => group_value(lookup, name),
        Function::GetModelIds => Ok(json!(
            lookup.get_model_ids(name, arguments.exclude_team_models)
        )),
        Function::GetCandidateModelIdsForRoute => Ok(json!(
            lookup.candidate_model_ids(name.ok_or(Failure::Argument("model_name"))?, team)?
        )),
        Function::HasModelId => candidate_value(lookup, arguments),
        Function::ResolveModelNameFromModelId => Ok(json!(
            lookup.resolve_model_name(arguments.model_id.as_deref())?
        )),
        Function::GetModelNames => Ok(json!(lookup.get_model_names(team)?)),
        Function::GetModelList => {
            let rows = lookup.get_model_list(name, team)?;
            Ok(Value::Array(rows.iter().map(deployment_row).collect()))
        }
        Function::GetModelListFromModelAlias => {
            let rows = lookup.list_from_alias(name)?;
            Ok(Value::Array(rows.iter().map(deployment_row).collect()))
        }
        Function::ShouldIncludeDeployment => include_answer(snapshot, arguments),
    }
}

fn group_value(lookup: &CatalogLookup<'_>, model_name: Option<&str>) -> Result<Value, Failure> {
    let row =
        lookup.deployment_by_group_name(model_name.ok_or(Failure::Argument("model_name"))?)?;
    Ok(row.map_or(Value::Null, deployment_row))
}

fn candidate_value(lookup: &CatalogLookup<'_>, arguments: &Arguments) -> Result<Value, Failure> {
    let candidate = arguments
        .candidate_id
        .as_deref()
        .ok_or(Failure::Argument("candidate_id"))?;
    Ok(json!(lookup.catalog.id_index.contains_key(candidate)))
}

// should_include_deployment reads the case's own row, so the caller picks it by position.
fn include_answer(snapshot: &Snapshot, arguments: &Arguments) -> Result<Value, Failure> {
    let model_name = arguments
        .model_name
        .as_deref()
        .ok_or(Failure::Argument("model_name"))?;
    let index = arguments
        .deployment_index
        .ok_or(Failure::Argument("deployment_index"))?;
    let deployment = snapshot
        .catalog
        .models
        .get(index)
        .ok_or(Failure::Argument("deployment_index"))?;
    Ok(json!(should_include_deployment(
        model_name,
        deployment,
        arguments.team_id.as_deref()
    )))
}

// One deployment as a case compares it: its names and its team ownership.
fn deployment_row(row: &DeploymentSpec) -> Value {
    json!({
        "model_name": row.model_name.as_str(),
        "model": text(&row.litellm_params, "model"),
        "id": text(&row.model_info, "id"),
        "team_id": text(&row.model_info, "team_id"),
        "public_name": text(&row.model_info, "team_public_model_name"),
    })
}

fn text<'value>(map: &'value JsonObject, key: &str) -> Option<&'value str> {
    match map.get(key) {
        Some(Json::Text(value)) => Some(value),
        _ => None,
    }
}
