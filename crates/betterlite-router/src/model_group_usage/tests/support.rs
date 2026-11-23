use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;
use tokio::runtime::{Builder, Runtime};

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::alias::AliasMap;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::model_group_info::group::ModelGroupInfo;
use crate::model_group_usage::access_filter::KeyGrants;
use crate::model_group_usage::counters::UsageCounterCache;
use crate::model_group_usage::metadata::GroupMetadata;
use crate::remaining_usage_headers::RemainingUsage;
use crate::routing_groups::index::RoutingGroupIndex;

const NOW_SECONDS: u64 = 3660;

#[derive(Clone, Debug, thiserror::Error)]
#[error("the counter read failed")]
pub(crate) struct CacheFailure;

#[derive(Clone, Debug, thiserror::Error)]
#[error("the group metadata read failed")]
pub(crate) struct MetadataFailure;

pub(crate) struct FakeCache {
    answer: Result<Option<Vec<Option<Json>>>, CacheFailure>,
    batches: Mutex<Vec<Vec<String>>>,
}

impl FakeCache {
    pub(crate) fn answering(values: Option<Vec<Option<Json>>>) -> Self {
        Self {
            answer: Ok(values),
            batches: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            answer: Err(CacheFailure),
            batches: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn batches(&self) -> Vec<Vec<String>> {
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl UsageCounterCache for FakeCache {
    type Error = CacheFailure;

    async fn batch_get(&self, keys: &[String]) -> Result<Option<Vec<Option<Json>>>, CacheFailure> {
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(keys.to_vec());
        self.answer.clone()
    }
}

pub(crate) struct FakeMetadata {
    pub(crate) answer: Option<ModelGroupInfo>,
}

#[async_trait]
impl GroupMetadata for FakeMetadata {
    type Error = MetadataFailure;

    async fn cached_group_info(
        &self,
        _model_group: &str,
    ) -> Result<Option<ModelGroupInfo>, MetadataFailure> {
        Ok(self.answer.clone())
    }
}

pub(crate) struct FailingMetadata;

#[async_trait]
impl GroupMetadata for FailingMetadata {
    type Error = MetadataFailure;

    async fn cached_group_info(
        &self,
        _model_group: &str,
    ) -> Result<Option<ModelGroupInfo>, MetadataFailure> {
        Err(MetadataFailure)
    }
}

pub(crate) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds")
}

pub(crate) fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(NOW_SECONDS))
        .expect("the test clock")
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(crate) fn answered(values: Vec<Json>) -> Option<Vec<Option<Json>>> {
    Some(values.into_iter().map(Some).collect())
}

pub(crate) fn float(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("the fixture float"))
}

pub(crate) fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn generated_id() -> String {
    "generated".to_owned()
}

pub(crate) fn deployment(group: &str, id: &str, model: &str) -> DeploymentSpec {
    DeploymentSpec::new(
        group.to_owned(),
        object(vec![("model", text(model))]),
        object(vec![("id", text(id))]),
        JsonObject::new(),
        generated_id,
    )
    .expect("the fixture deployment is invalid")
}

pub(crate) fn grouped(group: &str, id: &str, groups: &[&str]) -> DeploymentSpec {
    let listed: Vec<Json> = groups.iter().map(|group| text(group)).collect();
    DeploymentSpec::new(
        group.to_owned(),
        object(vec![("model", text("openai/m"))]),
        object(vec![
            ("id", text(id)),
            ("access_groups", Json::List(listed)),
        ]),
        JsonObject::new(),
        generated_id,
    )
    .expect("the fixture deployment is invalid")
}

pub(crate) fn group_info(
    tpm: Option<i64>,
    rpm: Option<i64>,
    itpm: Option<i64>,
    otpm: Option<i64>,
) -> ModelGroupInfo {
    ModelGroupInfo {
        model_group: "group".to_owned(),
        providers: Vec::new(),
        max_input_tokens: None,
        max_output_tokens: None,
        input_cost_per_token: None,
        output_cost_per_token: None,
        input_cost_per_pixel: None,
        mode: None,
        tpm,
        rpm,
        itpm,
        otpm,
        capabilities: BTreeSet::new(),
        supports_fast_mode: false,
        supported_reasoning_efforts: None,
        supported_openai_params: None,
        clientside_auth: None,
    }
}

pub(crate) fn grants(models: &[&str], team_models: &[&str]) -> KeyGrants {
    KeyGrants {
        models: models.iter().map(|model| (*model).to_owned()).collect(),
        team_models: team_models
            .iter()
            .map(|model| (*model).to_owned())
            .collect(),
    }
}

const fn no_inferred(_model: &str) -> Option<String> {
    None
}

pub(crate) struct World {
    catalog: DeploymentCatalog,
    aliases: AliasMap,
    groups: RoutingGroupIndex,
}

impl World {
    pub(crate) fn new(rows: &[DeploymentSpec]) -> Self {
        let mut world = Self {
            catalog: DeploymentCatalog::default(),
            aliases: AliasMap::new(),
            groups: RoutingGroupIndex::default(),
        };
        for row in rows {
            world
                .catalog
                .index_patterns(row)
                .expect("the fixture pattern index");
            world.catalog.push(row.clone());
        }
        world
    }

    pub(crate) fn lookup(&self) -> CatalogLookup<'_> {
        CatalogLookup {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: &no_inferred,
        }
    }
}

pub(crate) fn tpm_key(id: &str, model: &str) -> String {
    format!("global_router:{id}:{model}:tpm:01-01")
}

pub(crate) fn rpm_key(id: &str, model: &str) -> String {
    format!("global_router:{id}:{model}:rpm:01-01")
}

pub(crate) fn itpm_key(id: &str, model: &str) -> String {
    format!("global_router:{id}:{model}:itpm:01-01")
}

pub(crate) fn otpm_key(id: &str, model: &str) -> String {
    format!("global_router:{id}:{model}:otpm:01-01")
}

pub(crate) fn ordered(headers: &RemainingUsage) -> Vec<(&str, i64)> {
    headers
        .iter()
        .map(|(header, value)| (header.as_str(), *value))
        .collect()
}
