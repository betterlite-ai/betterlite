use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_read::table::AuthReadError;
use betterlite_auth::auth_tag_budgets::cache::{CachedValue, TagBudgetCache};
use betterlite_auth::auth_tag_budgets::groups::ModelAccessGroupBudget;
use betterlite_auth::auth_tag_budgets::rows::{
    BudgetRows, ModelAccessGroupBudgetRow, TagBudgetRowsFailure,
};
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_models::decisions::Json;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{Function, Input};

// LiteLLM's management-object cache default, which every row and registry write carries
const MANAGEMENT_TTL_SECONDS: f64 = 60.0;
const TAG_PREFIX: &str = "tag:";
const GROUP_PREFIX: &str = "model_access_group:";

// every cache and row effect of one case, in the order the production module made it
#[derive(Clone, Default)]
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
        if matches!(&input.function, Function::Tags | Function::TagRegistry) {
            seed(
                &mut fixture.tags,
                &mut fixture.tag_rows,
                input,
                |tag: &TagTable| tag.tag_name.clone(),
                |name| json!({"tag_name": name, "models": [], "spend": 0}),
            )?;
        } else {
            seed(
                &mut fixture.groups,
                &mut fixture.group_rows,
                input,
                |budget: &ModelAccessGroupBudget| budget.access_group_name.clone(),
                |name| json!({"access_group_name": name, "spend": 0}),
            )?;
        }
        Ok(fixture)
    }

    pub(super) fn tracked(&self) -> MutexGuard<'_, Effects> {
        match self.effects.lock() {
            Ok(effects) => effects,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn probe<Entry>(
        &self,
        table: &BTreeMap<String, Entry>,
        key: &str,
        prefix: &str,
    ) -> Option<Entry>
    where
        Entry: Clone,
    {
        self.tracked().reads.push(key.to_owned());
        table.get(key.strip_prefix(prefix).unwrap_or(key)).cloned()
    }
}

// what a case scripts: cached values, database rows, and one row per generated name
fn seed<Cached, Row>(
    cached: &mut BTreeMap<String, Cached>,
    rows: &mut Vec<Row>,
    input: &Input,
    name: fn(&Cached) -> String,
    wire: fn(&str) -> Value,
) -> Result<(), serde_json::Error>
where
    Cached: DeserializeOwned,
    Row: DeserializeOwned,
{
    for value in &input.cached {
        let item: Cached = serde_json::from_value(value.clone())?;
        let _previous = cached.insert(name(&item), item);
    }
    for value in &input.rows {
        rows.push(serde_json::from_value(value.clone())?);
    }
    if let Some(generated) = &input.generated {
        for index in 0..generated.count {
            let generated_name = format!("{}-{index}", generated.prefix);
            rows.push(serde_json::from_value(wire(&generated_name))?);
        }
    }
    Ok(())
}

// the one failure a fake database raises: a query that could not reach a database
fn failure() -> TagBudgetRowsFailure {
    TagBudgetRowsFailure {
        source: Box::new(AuthReadError::Repository(DbError::NoDbConnected)),
    }
}

#[async_trait]
impl TagBudgetCache for Fixture {
    async fn read_tag(&self, key: &str) -> Result<Option<TagTable>, PayloadError> {
        Ok(self.probe(&self.tags, key, TAG_PREFIX))
    }

    async fn read_group(&self, key: &str) -> Result<Option<ModelAccessGroupBudget>, PayloadError> {
        Ok(self.probe(&self.groups, key, GROUP_PREFIX))
    }

    // the fixture starts with no cached registry, so every load queries the rows
    async fn read_registry(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
        self.tracked().reads.push(key.to_owned());
        Ok(None)
    }

    async fn write(
        &self,
        key: &str,
        _value: CachedValue<'_>,
        ttl: f64,
    ) -> Result<(), AuthCacheError> {
        self.tracked().writes.push((key.to_owned(), ttl));
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        MANAGEMENT_TTL_SECONDS
    }
}

macro_rules! fixture_rows {
    ($model:ty, $rows:ident, $name:ident) => {
        #[async_trait]
        impl BudgetRows<$model> for Fixture {
            async fn by_names(
                &self,
                names: Vec<String>,
            ) -> Result<Vec<$model>, TagBudgetRowsFailure> {
                self.tracked().queries.push(names.to_vec());
                if self.fail_query {
                    return Err(failure());
                }
                Ok(self
                    .$rows
                    .iter()
                    .filter(|row| {
                        names
                            .iter()
                            .any(|wanted| row.$name.as_str() == wanted.as_str())
                    })
                    .cloned()
                    .collect())
            }

            async fn registry_rows(&self, take: i64) -> Result<Vec<$model>, TagBudgetRowsFailure> {
                self.tracked().takes.push(take);
                if self.fail_query {
                    return Err(failure());
                }
                let limit = usize::try_from(take).unwrap_or(usize::MAX);
                Ok(self.$rows.iter().take(limit).cloned().collect())
            }
        }
    };
}

fixture_rows!(TagTable, tag_rows, tag_name);
fixture_rows!(ModelAccessGroupBudgetRow, group_rows, access_group_name);
