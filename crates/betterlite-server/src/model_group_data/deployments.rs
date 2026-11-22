use std::collections::BTreeSet;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use tracing::debug;

use super::errors::{GroupDataError, HttpFailure};
use super::ports::GroupDatabase;
use super::types::DeploymentRow;
use crate::model_reload::model_info::model_info_as_mapping;

// access_groups as stored: absent starts empty, and a value of another type makes the write refuse instead of repairing it.
fn access_groups(model_info: &JsonObject) -> Result<Vec<Json>, GroupDataError> {
    match model_info.get("access_groups") {
        None => Ok(Vec::new()),
        Some(Json::List(items)) => Ok(items.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(GroupDataError::Runtime {
                message: "access_groups is not a list; refusing to rewrite it".to_owned(),
            })
        }
    }
}

// add_access_group_to_deployment: append the group once, and report whether a write is needed.
pub fn add_access_group_to_deployment(
    model_info: &mut JsonObject,
    access_group: &str,
) -> Result<bool, GroupDataError> {
    let mut groups = access_groups(model_info)?;
    if groups
        .iter()
        .any(|item| matches!(item, Json::Text(text) if text == access_group))
    {
        return Ok(false);
    }
    groups.push(Json::Text(access_group.to_owned()));
    model_info.insert("access_groups".to_owned(), Json::List(groups));
    Ok(true)
}

// remove_access_group_from_deployment: drop the first occurrence only, like list.remove.
pub fn remove_group_from_deployment(
    model_info: &mut JsonObject,
    access_group: &str,
) -> Result<bool, GroupDataError> {
    let mut groups = access_groups(model_info)?;
    let Some(index) = groups
        .iter()
        .position(|item| matches!(item, Json::Text(text) if text == access_group))
    else {
        return Ok(false);
    };
    groups.remove(index);
    model_info.insert("access_groups".to_owned(), Json::List(groups));
    Ok(true)
}

// _readable_model_info_or_raise: an absent value starts empty, a present unreadable one refuses.
pub fn readable_model_info_or_raise(
    model_id: &str,
    model_info: Option<&Json>,
) -> Result<JsonObject, GroupDataError> {
    match model_info {
        None | Some(Json::Null) => Ok(JsonObject::new()),
        Some(value) => {
            let Some(mapping) = model_info_as_mapping(value) else {
                return Err(GroupDataError::ValueError {
                    message: format!(
                        "model_info for deployment {model_id} is not a readable JSON object; refusing to rewrite it"
                    ),
                });
            };
            Ok(mapping)
        }
    }
}

// The two directions a deployment's access_groups list changes.
#[derive(Clone, Copy)]
pub enum GroupChange {
    Add,
    Remove,
}

// _tag_deployment_with_access_group and _strip_access_group_from_deployment: one write path, and the pair comes back only when this call wrote the row.
pub async fn update_deployment_group<Database>(
    model_id: &str,
    model_info: Option<&Json>,
    access_group: &str,
    change: GroupChange,
    database: &Database,
) -> Result<Option<(String, JsonObject)>, GroupDataError>
where
    Database: GroupDatabase + ?Sized,
{
    let mut updated = readable_model_info_or_raise(model_id, model_info)?;
    let modified = match change {
        GroupChange::Add => add_access_group_to_deployment(&mut updated, access_group)?,
        GroupChange::Remove => remove_group_from_deployment(&mut updated, access_group)?,
    };
    if !modified {
        return Ok(None);
    }
    let serialized =
        serde_json::to_string(&Json::Object(updated.clone())).map_err(GroupDataError::Row)?;
    database.update_model_info(model_id, &serialized).await?;
    if matches!(change, GroupChange::Add) {
        debug!(
            model_id,
            access_group, "Updated deployment with access group"
        );
    }
    Ok(Some((model_id.to_owned(), updated)))
}

// _find_deployment_or_400: the row's model_info, or the 400 the id-facing update answers with.
pub async fn find_deployment_info_or_400<Database>(
    model_id: &str,
    database: &Database,
) -> Result<Option<Json>, GroupDataError>
where
    Database: GroupDatabase + ?Sized,
{
    let found = database.deployment_find_unique(model_id).await?;
    match found {
        Some(deployment) => Ok(deployment.model_info),
        None => Err(GroupDataError::Http(HttpFailure {
            status: StatusCode::BAD_REQUEST,
            message: format!("Deployment with model_id '{model_id}' not found in Database."),
        })),
    }
}

// The two selection modes of update_deployments_with_access_group and update_specific_deployments_with_access_group.
pub enum DeploymentSelection<'a> {
    ByNames(&'a [String]),
    ByIds(&'a [String]),
}

// update_deployments_with_access_group and update_specific_deployments_with_access_group: every name is checked before the first write, while each id is re-read just before its own write.
pub async fn tag_deployments<Database>(
    selection: DeploymentSelection<'_>,
    access_group: &str,
    database: &Database,
) -> Result<Vec<(String, JsonObject)>, GroupDataError>
where
    Database: GroupDatabase + ?Sized,
{
    match selection {
        DeploymentSelection::ByNames(model_names) => {
            let deployments = named_deployments(model_names, database).await?;
            let mut tagged = Vec::new();
            for deployment in &deployments {
                let pair = update_deployment_group(
                    &deployment.model_id,
                    deployment.model_info.as_ref(),
                    access_group,
                    GroupChange::Add,
                    database,
                )
                .await?;
                tagged.extend(pair);
            }
            Ok(tagged)
        }
        DeploymentSelection::ByIds(model_ids) => {
            debug!(model_ids = ?model_ids, "Updating specific deployment model ids");
            let mut tagged = Vec::new();
            for model_id in model_ids {
                tagged.extend(tag_one_deployment(model_id, access_group, database).await?);
            }
            Ok(tagged)
        }
    }
}

// The by-name rows, once every requested name has been found so a missing one writes nothing.
async fn named_deployments<Database>(
    model_names: &[String],
    database: &Database,
) -> Result<Vec<DeploymentRow>, GroupDataError>
where
    Database: GroupDatabase + ?Sized,
{
    let deployments = database.deployment_find_many(Some(model_names)).await?;
    debug!(
        count = deployments.len(),
        model_names = ?model_names,
        "Found deployments for model names"
    );
    ensure_names_found(&deployments, model_names)?;
    Ok(deployments)
}

// The 400 for a missing model name, checked over every requested name before the first write.
fn ensure_names_found(
    deployments: &[DeploymentRow],
    model_names: &[String],
) -> Result<(), GroupDataError> {
    let found: BTreeSet<&str> = deployments
        .iter()
        .map(|deployment| deployment.model_name.as_str())
        .collect();
    for model_name in model_names {
        if !found.contains(model_name.as_str()) {
            return Err(GroupDataError::Http(HttpFailure {
                status: StatusCode::BAD_REQUEST,
                message: format!(
                    "Can't find model '{model_name}' in Database. Access group management is only supported for database models."
                ),
            }));
        }
    }
    Ok(())
}

// One id from update_specific_deployments_with_access_group: the row is read again just before its write, because an earlier duplicate may have changed it.
async fn tag_one_deployment<Database>(
    model_id: &str,
    access_group: &str,
    database: &Database,
) -> Result<Option<(String, JsonObject)>, GroupDataError>
where
    Database: GroupDatabase + ?Sized,
{
    let model_info = find_deployment_info_or_400(model_id, database).await?;
    update_deployment_group(
        model_id,
        model_info.as_ref(),
        access_group,
        GroupChange::Add,
        database,
    )
    .await
}
