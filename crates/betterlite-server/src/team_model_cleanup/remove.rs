use std::collections::BTreeSet;

use async_trait::async_trait;
use betterlite_db::team_repository::decoder::TeamRecord;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use serde::Deserialize;

use super::alias::{AliasRows, delete_team_model_alias};
use super::deployments::{DeploymentRows, team_public_model_names};
use super::error::CleanupError;
use crate::team_fields::rows::{TeamObjectCache, refresh_cached_team};

// The typed slice of a deleted deployment that _remove_unbacked_team_models reads: Deployment.model_name and its model_info.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct DeletedModel {
    pub model_name: String,
    pub team_id: Option<String>,
    pub team_public_model_name: Option<String>,
}

#[async_trait]
pub trait TeamModelRows: Send + Sync {
    async fn team_models(&self, team_id: &str) -> Result<Option<Vec<String>>, CleanupError>;

    async fn save_team_models(
        &self,
        team_id: &str,
        models: &[String],
    ) -> Result<TeamRecord, CleanupError>;
}

// llm_router.model_name_to_deployment_indices: a name still serving on this pod is not revoked.
pub trait ServingNames: Send + Sync {
    fn serves(&self, model_name: &str) -> bool;
}

pub struct RemovalContext<'context> {
    pub aliases: &'context dyn AliasRows,
    pub deployments: &'context dyn DeploymentRows,
    pub teams: &'context dyn TeamModelRows,
    pub serving: Option<&'context dyn ServingNames>,
    pub cache: &'context dyn TeamObjectCache,
}

impl ServingNames for DeploymentCatalog {
    fn serves(&self, model_name: &str) -> bool {
        self.name_index.contains_key(model_name)
    }
}

// _remove_unbacked_team_models: strip the public names no remaining deployment backs, then refresh the team cache.
pub async fn remove_unbacked_team_models(
    model: &DeletedModel,
    context: &RemovalContext<'_>,
) -> Result<Option<TeamRecord>, CleanupError> {
    let Some(team_id) = model.team_id.as_deref() else {
        return Ok(None);
    };
    let removed = if context
        .serving
        .is_some_and(|names| names.serves(&model.model_name))
    {
        Vec::new()
    } else {
        delete_team_model_alias(&model.model_name, context.aliases).await?
    };
    let candidates = candidate_names(&removed, team_id, model.team_public_model_name.as_deref());
    if candidates.is_empty() {
        return Ok(None);
    }
    let backed = team_public_model_names(team_id, context.deployments).await?;
    let to_remove = unbacked_names(&candidates, &backed, context.serving);
    if to_remove.is_empty() {
        return Ok(None);
    }
    let Some(models) = context.teams.team_models(team_id).await? else {
        return Ok(None);
    };
    let kept: Vec<String> = models
        .into_iter()
        .filter(|name| !to_remove.contains(name))
        .collect();
    let updated = context.teams.save_team_models(team_id, &kept).await?;
    refresh_cached_team(&updated, context.cache).await?;
    Ok(Some(updated))
}

fn candidate_names(
    removed: &[(String, String)],
    team_id: &str,
    public_name: Option<&str>,
) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for (alias_team, alias) in removed {
        if alias_team == team_id {
            names.insert(alias.clone());
        }
    }
    if let Some(name) = public_name {
        names.insert(name.to_owned());
    }
    names
}

fn unbacked_names(
    candidates: &BTreeSet<String>,
    backed: &BTreeSet<String>,
    serving: Option<&dyn ServingNames>,
) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for name in candidates {
        if !backed.contains(name) && !serving.is_some_and(|names| names.serves(name)) {
            found.insert(name.clone());
        }
    }
    found
}
