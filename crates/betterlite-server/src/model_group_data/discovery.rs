use std::collections::{BTreeMap, BTreeSet};

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::errors::{GroupDataError, HttpFailure};
use super::ports::{GroupDatabase, ModelNameSource};
use super::types::{AccessGroupInfo, DeploymentRow};
use crate::route_read_through::ModelReadThrough;

// A stored model_info as discovery reads it: only a JSON object carries tags, with no parse of the text form the write paths accept.
fn tagged_model_info(deployment: &DeploymentRow) -> Result<JsonObject, GroupDataError> {
    match &deployment.model_info {
        None | Some(Json::Null) => Ok(JsonObject::new()),
        Some(Json::Object(mapping)) => Ok(mapping.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(GroupDataError::Runtime {
                message: format!(
                    "model_info for deployment {} is not a JSON object",
                    deployment.model_id
                ),
            })
        }
    }
}

fn deployment_access_groups(deployment: &DeploymentRow) -> Result<Vec<String>, GroupDataError> {
    let model_info = tagged_model_info(deployment)?;
    match model_info.get("access_groups") {
        None => Ok(Vec::new()),
        Some(Json::List(items)) => {
            let mut groups = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Json::Text(group) => groups.push(group.clone()),
                    Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::List(_)
                    | Json::Object(_) => {
                        return Err(GroupDataError::Runtime {
                            message: format!(
                                "access_groups for deployment {} holds a non-string entry",
                                deployment.model_id
                            ),
                        });
                    }
                }
            }
            Ok(groups)
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(GroupDataError::Runtime {
                message: format!(
                    "access_groups for deployment {} is not a list",
                    deployment.model_id
                ),
            })
        }
    }
}

// get_all_access_groups_from_db: every deployment's tags, with duplicates counted per deployment.
pub async fn get_all_access_groups_from_db<Database>(
    database: &Database,
) -> Result<BTreeMap<String, AccessGroupInfo>, GroupDataError>
where
    Database: GroupDatabase + ?Sized,
{
    let deployments = database.deployment_find_many(None).await?;
    let mut names_by_group: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for deployment in &deployments {
        let groups = deployment_access_groups(deployment)?;
        for group in groups {
            names_by_group
                .entry(group)
                .or_default()
                .push(deployment.model_name.clone());
        }
    }
    let mut access_groups = BTreeMap::new();
    for (access_group, mut model_names) in names_by_group {
        let deployment_count = model_names.len();
        model_names.sort_unstable();
        model_names.dedup();
        access_groups.insert(
            access_group.clone(),
            AccessGroupInfo {
                access_group,
                model_names,
                deployment_count,
                spend: None,
                budget: None,
            },
        );
    }
    Ok(access_groups)
}

// _raise_404_if_model_access_group_missing: existence comes from the deployment tags alone.
pub async fn require_group_exists<Database>(
    access_group: &str,
    database: &Database,
) -> Result<(), GroupDataError>
where
    Database: GroupDatabase + ?Sized,
{
    let access_groups = get_all_access_groups_from_db(database).await?;
    if access_groups.contains_key(access_group) {
        return Ok(());
    }
    Err(GroupDataError::Http(HttpFailure {
        status: StatusCode::NOT_FOUND,
        message: format!("Access group '{access_group}' not found"),
    }))
}

// validate_models_exist: exact name matches only, and no router reports every name as missing.
pub fn validate_models_exist<Source>(
    model_names: &[String],
    source: Option<&Source>,
) -> Result<(bool, Vec<String>), GroupDataError>
where
    Source: ModelNameSource + ?Sized,
{
    let Some(installed) = source else {
        return Ok((false, model_names.to_vec()));
    };
    let Some(served) = installed.model_names()? else {
        return Ok((false, model_names.to_vec()));
    };
    let known: BTreeSet<String> = served.into_iter().collect();
    let mut missing = Vec::new();
    for name in model_names {
        if !known.contains(name) {
            missing.push(name.clone());
        }
    }
    Ok((missing.is_empty(), missing))
}

// _missing_models_after_read_through: attempt every initially missing name, then recheck all names against the router, which a read-through may have installed.
pub async fn missing_after_read_through<Source>(
    model_names: &[String],
    source: &Source,
    read_through: Option<&dyn ModelReadThrough>,
) -> Result<Vec<String>, GroupDataError>
where
    Source: ModelNameSource + ?Sized,
{
    let (_, missing) = validate_models_exist(model_names, Some(source))?;
    if missing.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(reader) = read_through {
        for name in &missing {
            reader.attempt_model(name).await;
        }
    }
    let (_, still_missing) = validate_models_exist(model_names, Some(source))?;
    Ok(still_missing)
}
