use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_repository::decoder::TeamRecord;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use serde_json::from_str;

use crate::team_fields::rows::TeamObjectCache;
use crate::team_model_cleanup::alias::{AliasMap, AliasRow, AliasRows};
use crate::team_model_cleanup::deployments::{DeploymentRow, DeploymentRows};
use crate::team_model_cleanup::error::CleanupError;
use crate::team_model_cleanup::remove::{RemovalContext, ServingNames, TeamModelRows};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn value(text: &str) -> Json {
    from_str(text).expect("the fixture is JSON")
}

pub(crate) fn aliases(text: &str) -> AliasMap {
    from_str::<JsonObject>(text).expect("the fixture is a JSON object")
}

pub(crate) fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

pub(crate) fn alias_row(id: i32, team_id: Option<&str>, stored: Option<&str>) -> AliasRow {
    AliasRow {
        id,
        team_id: team_id.map(str::to_owned),
        aliases: stored.map(aliases),
    }
}

pub(crate) fn deployment(model_name: &str, model_info: &str) -> DeploymentRow {
    DeploymentRow {
        model_name: model_name.to_owned(),
        model_info: value(model_info),
    }
}

#[derive(Default)]
pub(crate) struct FakeStore {
    pub(crate) aliases: Mutex<Vec<AliasRow>>,
    pub(crate) deployments: Vec<DeploymentRow>,
    pub(crate) team_id: String,
    pub(crate) team_models: Mutex<Option<Vec<String>>>,
    pub(crate) saved_aliases: Mutex<Vec<(i32, AliasMap)>>,
    pub(crate) saved_models: Mutex<Vec<Vec<String>>>,
}

impl FakeStore {
    pub(crate) fn new(
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
            saved_aliases: Mutex::new(Vec::new()),
            saved_models: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn remaining_aliases(&self) -> Vec<Option<AliasMap>> {
        lock(&self.aliases)
            .iter()
            .map(|row| row.aliases.clone())
            .collect()
    }

    pub(crate) fn current_models(&self) -> Option<Vec<String>> {
        lock(&self.team_models).clone()
    }
}

#[async_trait]
impl AliasRows for FakeStore {
    async fn alias_rows(&self) -> Result<Vec<AliasRow>, CleanupError> {
        Ok(lock(&self.aliases).clone())
    }

    async fn save_aliases(&self, id: i32, aliases: &AliasMap) -> Result<(), CleanupError> {
        lock(&self.saved_aliases).push((id, aliases.clone()));
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
        lock(&self.saved_models).push(models.to_vec());
        *lock(&self.team_models) = Some(models.to_vec());
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

#[derive(Default)]
pub(crate) struct FakeServing {
    pub(crate) names: Vec<String>,
}

impl ServingNames for FakeServing {
    fn serves(&self, model_name: &str) -> bool {
        self.names.iter().any(|name| name == model_name)
    }
}

pub(crate) fn serving(names: &[&str]) -> FakeServing {
    FakeServing {
        names: names.iter().map(|name| (*name).to_owned()).collect(),
    }
}

pub(crate) fn context<'ctx>(
    store: &'ctx FakeStore,
    cache: &'ctx FakeCache,
    serving_names: Option<&'ctx dyn ServingNames>,
) -> RemovalContext<'ctx> {
    RemovalContext {
        aliases: store,
        deployments: store,
        teams: store,
        serving: serving_names,
        cache,
    }
}

#[derive(Default)]
pub(crate) struct FakeCache {
    pub(crate) stored: Mutex<Vec<(String, Vec<String>)>>,
}

impl FakeCache {
    pub(crate) fn stored(&self) -> Vec<(String, Vec<String>)> {
        lock(&self.stored).clone()
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
