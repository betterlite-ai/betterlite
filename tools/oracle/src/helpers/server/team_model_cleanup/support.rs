use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_repository::decoder::TeamRecord;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_server::team_fields::rows::TeamObjectCache;
use betterlite_server::team_model_cleanup::alias::{AliasMap, AliasRow, AliasRows};
use betterlite_server::team_model_cleanup::deployments::{DeploymentRow, DeploymentRows};
use betterlite_server::team_model_cleanup::error::CleanupError;
use betterlite_server::team_model_cleanup::remove::{RemovalContext, ServingNames, TeamModelRows};
use indexmap::IndexMap;
use serde_json::{Value, json};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) struct FakeStore {
    pub(super) aliases: Mutex<Vec<AliasRow>>,
    pub(super) deployments: Vec<DeploymentRow>,
    pub(super) team_id: String,
    pub(super) team_models: Mutex<Option<Vec<String>>>,
}

impl FakeStore {
    pub(super) fn new(
        team_id: &str,
        team_models: Option<Vec<String>>,
        aliases: Vec<AliasRow>,
        deployments: Vec<DeploymentRow>,
    ) -> Self {
        Self {
            aliases: Mutex::new(aliases),
            deployments,
            team_id: team_id.to_owned(),
            team_models: Mutex::new(team_models),
        }
    }

    pub(super) fn alias_summary(&self) -> Vec<Value> {
        lock(&self.aliases)
            .iter()
            .map(|row| json!({ "id": row.id, "aliases": row.aliases }))
            .collect()
    }

    pub(super) fn current_models(&self) -> Option<Vec<String>> {
        lock(&self.team_models).clone()
    }
}

#[async_trait]
impl AliasRows for FakeStore {
    async fn alias_rows(&self) -> Result<Vec<AliasRow>, CleanupError> {
        Ok(lock(&self.aliases).clone())
    }

    async fn save_aliases(&self, id: i64, aliases: &AliasMap) -> Result<(), CleanupError> {
        for row in lock(&self.aliases).iter_mut() {
            if row.id == id {
                row.aliases = Some(aliases.clone());
            }
        }
        Ok(())
    }
}

#[async_trait]
impl DeploymentRows for FakeStore {
    async fn deployments_with_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<DeploymentRow>, CleanupError> {
        Ok(self
            .deployments
            .iter()
            .filter(|row| row.model_name.starts_with(prefix))
            .cloned()
            .collect())
    }
}

#[async_trait]
impl TeamModelRows for FakeStore {
    async fn team_models(&self, team_id: &str) -> Result<Option<Vec<String>>, CleanupError> {
        if team_id == self.team_id {
            return Ok(self.current_models());
        }
        Ok(None)
    }

    async fn save_team_models(
        &self,
        team_id: &str,
        models: &[String],
    ) -> Result<TeamRecord, CleanupError> {
        let mut guard = lock(&self.team_models);
        *guard = Some(models.to_vec());
        drop(guard);
        Ok(record(team_id, models))
    }
}

fn record(team_id: &str, models: &[String]) -> TeamRecord {
    let mut fields = IndexMap::new();
    fields.insert("team_id".to_owned(), JsonData::String(team_id.to_owned()));
    fields.insert(
        "models".to_owned(),
        JsonData::Array(
            models
                .iter()
                .map(|name| JsonData::String(name.clone()))
                .collect(),
        ),
    );
    TeamRecord { fields }
}

pub(super) struct FakeServing {
    names: Vec<String>,
}

impl FakeServing {
    pub(super) fn new(names: &[String]) -> Self {
        Self {
            names: names.to_vec(),
        }
    }
}

impl ServingNames for FakeServing {
    fn serves(&self, model_name: &str) -> bool {
        self.names.iter().any(|name| name == model_name)
    }
}

#[derive(Default)]
pub(super) struct FakeCache {
    stored: Mutex<Vec<(String, Vec<String>)>>,
}

impl FakeCache {
    pub(super) fn summary(&self) -> Value {
        match lock(&self.stored).last() {
            Some((team_id, models)) => json!({ "team_id": team_id, "models": models }),
            None => Value::Null,
        }
    }
}

#[async_trait]
impl TeamObjectCache for FakeCache {
    async fn store_team(
        &self,
        team_id: &str,
        team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), InvalidationError> {
        lock(&self.stored).push((team_id.to_owned(), team.team.fields.base.models.clone()));
        Ok(())
    }
}

pub(super) fn context<'context>(
    store: &'context FakeStore,
    cache: &'context FakeCache,
    serving: Option<&'context dyn ServingNames>,
) -> RemovalContext<'context> {
    RemovalContext {
        aliases: store,
        deployments: store,
        teams: store,
        serving,
        cache,
    }
}
