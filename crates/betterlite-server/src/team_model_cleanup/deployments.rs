use std::collections::BTreeSet;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use serde::Deserialize;

use super::error::CleanupError;
use crate::model_reload::model_info::model_info_as_mapping;

// One row of litellm_proxymodeltable as _get_team_deployments reads it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct DeploymentRow {
    pub model_name: String,
    pub model_info: Json,
}

#[async_trait]
pub trait DeploymentRows: Send + Sync {
    async fn deployments_with_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<DeploymentRow>, CleanupError>;
}

// _get_team_deployments: the prefix narrows the query, and model_info.team_id confirms each row.
pub async fn team_deployments(
    team_id: &str,
    rows: &dyn DeploymentRows,
) -> Result<Vec<DeploymentRow>, CleanupError> {
    let prefix = format!("model_name_{team_id}_");
    let stored = rows.deployments_with_prefix(&prefix).await?;
    Ok(stored
        .into_iter()
        .filter(|row| belongs_to_team(&row.model_info, team_id))
        .collect())
}

// _get_team_public_model_names: every truthy public name a remaining deployment backs.
pub async fn team_public_model_names(
    team_id: &str,
    rows: &dyn DeploymentRows,
) -> Result<BTreeSet<String>, CleanupError> {
    let mut names = BTreeSet::new();
    for row in team_deployments(team_id, rows).await? {
        if let Some(name) = public_model_name(&row.model_info) {
            names.insert(name);
        }
    }
    Ok(names)
}

fn belongs_to_team(model_info: &Json, team_id: &str) -> bool {
    let stored =
        model_info_as_mapping(model_info).and_then(|mapping| mapping.get("team_id").cloned());
    match stored {
        Some(Json::Text(found)) => found == team_id,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => false,
    }
}

fn public_model_name(model_info: &Json) -> Option<String> {
    let stored = model_info_as_mapping(model_info)
        .and_then(|mapping| mapping.get("team_public_model_name").cloned());
    match stored {
        Some(Json::Text(name)) if !name.is_empty() => Some(name),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    }
}
