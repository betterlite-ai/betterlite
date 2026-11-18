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
    let lookup = snapshot.lookup();
    let arguments = &input.arguments;
    let name = arguments.model_name.as_deref();
    let team = arguments.team_id.as_deref();
    let model_id = arguments.model_id.as_deref();
    match input.function {
        Function::GetDeployment => Ok(row_value(
            lookup.get_deployment(required(model_id, "model_id")?)?,
        )),
        Function::GetDeploymentByModelGroupName => Ok(row_value(
            lookup.get_deployment_by_model_group_name(required(name, "model_name")?)?,
        )),
        Function::GetModelIds => model_ids_answer(&lookup, arguments),
        Function::GetCandidateModelIdsForRoute => Ok(json!(
            lookup.get_candidate_model_ids_for_route(required(name, "model_name")?, team)?
        )),
        Function::HasModelId => Ok(json!(
            lookup.has_model_id(required(arguments.candidate_id.as_deref(), "candidate_id")?)
        )),
        Function::ResolveModelNameFromModelId => {
            Ok(json!(lookup.resolve_model_name_from_model_id(model_id)?))
        }
        Function::GetModelNames => Ok(json!(lookup.get_model_names(team)?)),
        Function::GetModelList => Ok(rows_value(&lookup.get_model_list(name, team)?)),
        Function::GetModelListFromModelAlias => {
            Ok(rows_value(&lookup.get_model_list_from_model_alias(name)?))
        }
        Function::ShouldIncludeDeployment => include_answer(&snapshot, arguments),
    }
}

fn model_ids_answer(lookup: &CatalogLookup<'_>, arguments: &Arguments) -> Result<Value, Failure> {
    let exclude = required_bool(arguments.exclude_team_models, "exclude_team_models")?;
    Ok(json!(
        lookup.get_model_ids(arguments.model_name.as_deref(), exclude)
    ))
}

// should_include_deployment reads the case's own row, so the caller picks it by position.
fn include_answer(snapshot: &Snapshot, arguments: &Arguments) -> Result<Value, Failure> {
    let model_name = required(arguments.model_name.as_deref(), "model_name")?;
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

fn required<'value>(
    value: Option<&'value str>,
    name: &'static str,
) -> Result<&'value str, Failure> {
    value.ok_or(Failure::Argument(name))
}

fn required_bool(value: Option<bool>, name: &'static str) -> Result<bool, Failure> {
    value.ok_or(Failure::Argument(name))
}

fn row_value(row: Option<&DeploymentSpec>) -> Value {
    row.map_or(Value::Null, deployment_row)
}

fn rows_value(rows: &[DeploymentSpec]) -> Value {
    Value::Array(rows.iter().map(deployment_row).collect())
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
