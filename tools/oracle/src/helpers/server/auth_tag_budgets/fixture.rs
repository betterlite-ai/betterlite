use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_read::table::AuthReadError;
use betterlite_auth::auth_tag_budgets::cache::TagBudgetCache;
use betterlite_auth::auth_tag_budgets::groups::ModelAccessGroupBudget;
use betterlite_auth::auth_tag_budgets::rows::{
    GroupRows, ModelAccessGroupBudgetRow, TagBudgetRowsFailure, TagRows,
};
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_models::decisions::Json;
use serde_json::json;

use super::{Generated, Input};

// LiteLLM's management-object cache default, which every row and registry write carries
const MANAGEMENT_TTL_SECONDS: f64 = 60.0;
const TAG_PREFIX: &str = "tag:";
const GROUP_PREFIX: &str = "model_access_group:";

// every cache and row effect of one case, in the order the production module made it
#[derive(Default)]
pub(super) struct Effects {
    pub(super) reads: Vec<String>,
    pub(super) writes: Vec<(String, f64)>,
    pub(super) queries: Vec<Vec<String>>,
    pub(super) takes: Vec<i64>,
}

pub(super) struct Fixture {
    effects: Mutex<Effects>,
    tags: BTreeMap<String, TagTable>,
    groups: BTreeMap<String, ModelAccessGroupBudget>,
    tag_rows: Vec<TagTable>,
    group_rows: Vec<ModelAccessGroupBudgetRow>,
    fail_query: bool,
}

fn generated_names(generated: Option<&Generated>) -> Vec<String> {
    generated.map_or_else(Vec::new, |generated| {
        (0..generated.count)
            .map(|index| format!("{}-{index}", generated.prefix))
            .collect()
    })
}

impl Fixture {
    pub(super) fn new(input: &Input) -> Result<Self, serde_json::Error> {
        let mut fixture = Self {
            effects: Mutex::new(Effects::default()),
            tags: BTreeMap::new(),
            groups: BTreeMap::new(),
            tag_rows: Vec::new(),
            group_rows: Vec::new(),
            fail_query: input.fail_query,
        };
        if input.function.tags() {
            for value in &input.cached {
                let tag: TagTable = serde_json::from_value(value.clone())?;
                let _previous = fixture.tags.insert(tag.tag_name.clone(), tag);
            }
            for value in &input.rows {
                fixture
                    .tag_rows
                    .push(serde_json::from_value(value.clone())?);
            }
            for name in generated_names(input.generated.as_ref()) {
                let row = json!({"tag_name": name, "models": [], "spend": 0});
                fixture.tag_rows.push(serde_json::from_value(row)?);
            }
        } else {
            for value in &input.cached {
                let budget: ModelAccessGroupBudget = serde_json::from_value(value.clone())?;
                let _previous = fixture
                    .groups
                    .insert(budget.access_group_name.clone(), budget);
            }
            for value in &input.rows {
                fixture
                    .group_rows
                    .push(serde_json::from_value(value.clone())?);
            }
            for name in generated_names(input.generated.as_ref()) {
                let row = json!({"access_group_name": name, "spend": 0});
                fixture.group_rows.push(serde_json::from_value(row)?);
            }
        }
        Ok(fixture)
    }

    pub(super) fn effects(&self) -> MutexGuard<'_, Effects> {
        self.effects.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn probe<Value: Clone>(
        &self,
        table: &BTreeMap<String, Value>,
        key: &str,
        prefix: &str,
    ) -> Option<Value> {
        self.effects().reads.push(key.to_owned());
        table.get(key.strip_prefix(prefix).unwrap_or(key)).cloned()
    }

    fn record_write(&self, key: &str, ttl: f64) {
        self.effects().writes.push((key.to_owned(), ttl));
    }
}

#[async_trait]
impl TagBudgetCache for Fixture {
    async fn read_tag(&self, key: &str) -> Result<Option<TagTable>, PayloadError> {
        Ok(self.probe(&self.tags, key, TAG_PREFIX))
    }

    async fn write_tag(&self, key: &str, _tag: &TagTable, ttl: f64) -> Result<(), AuthCacheError> {
        self.record_write(key, ttl);
        Ok(())
    }

    async fn read_group(&self, key: &str) -> Result<Option<ModelAccessGroupBudget>, PayloadError> {
        Ok(self.probe(&self.groups, key, GROUP_PREFIX))
    }

    async fn write_group(
        &self,
        key: &str,
        _budget: &ModelAccessGroupBudget,
        ttl: f64,
    ) -> Result<(), AuthCacheError> {
        self.record_write(key, ttl);
        Ok(())
    }

    // the fixture starts with no cached registry, so every load queries the rows
    async fn read_registry(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
        self.effects().reads.push(key.to_owned());
        Ok(None)
    }

    async fn write_registry(
        &self,
        key: &str,
        _answer: &Json,
        ttl: f64,
    ) -> Result<(), AuthCacheError> {
        self.record_write(key, ttl);
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        MANAGEMENT_TTL_SECONDS
    }
}

// the one failure a fake database raises: a query that could not reach a database
fn failure() -> TagBudgetRowsFailure {
    TagBudgetRowsFailure {
        source: Box::new(AuthReadError::Repository(DbError::NoDbConnected)),
    }
}

fn take_limit(take: i64) -> usize {
    usize::try_from(take).unwrap_or(usize::MAX)
}

#[async_trait]
impl TagRows for Fixture {
    async fn by_names(&self, names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        self.effects().queries.push(names.clone());
        if self.fail_query {
            return Err(failure());
        }
        Ok(self
            .tag_rows
            .iter()
            .filter(|row| names.contains(&row.tag_name))
            .cloned()
            .collect())
    }

    async fn registry_rows(&self, take: i64) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        self.effects().takes.push(take);
        if self.fail_query {
            return Err(failure());
        }
        Ok(self
            .tag_rows
            .iter()
            .take(take_limit(take))
            .cloned()
            .collect())
    }
}

#[async_trait]
impl GroupRows for Fixture {
    async fn by_names(
        &self,
        names: Vec<String>,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        self.effects().queries.push(names.clone());
        if self.fail_query {
            return Err(failure());
        }
        Ok(self
            .group_rows
            .iter()
            .filter(|row| names.contains(&row.access_group_name))
            .cloned()
            .collect())
    }

    async fn registry_rows(
        &self,
        take: i64,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        self.effects().takes.push(take);
        if self.fail_query {
            return Err(failure());
        }
        Ok(self
            .group_rows
            .iter()
            .take(take_limit(take))
            .cloned()
            .collect())
    }
}
