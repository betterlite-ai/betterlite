use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::keys::CacheKey;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_end_users::cache::{EndUserCache, EndUserCacheKey};
use betterlite_auth::auth_end_users::rows::{
    BudgetFetchError, BudgetRows, EndUserFetchError, EndUserRows,
};
use betterlite_auth::auth_read::table::AuthReadError;
use betterlite_auth::end_user_registry::cache::RegistryCache;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_models::decisions::Json;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{Failure, Input};

// the management-object ttl both sides write: the helper cache and the fixture's registry share it
const MANAGEMENT_TTL_SECONDS: f64 = 45.0;

// every cache and row effect of one case, in the order the production module made it
#[derive(Clone, Default)]
pub(super) struct Effects {
    pub(super) reads: Vec<String>,
    pub(super) writes: Vec<(String, f64)>,
    pub(super) queries: Vec<String>,
}

pub(super) struct Fixture {
    values: Mutex<BTreeMap<String, Value>>,
    effects: Mutex<Effects>,
    fail: Option<Failure>,
    budget_row: Option<LiteLLMBudgetTable>,
    end_user_row: Option<LiteLLMEndUserTable>,
}

impl Fixture {
    pub(super) fn new(input: &Input) -> Result<Self, serde_json::Error> {
        let mut values = BTreeMap::new();
        if let Some(budget) = &input.cached_budget {
            let id = budget.budget_id.as_deref().unwrap_or("None");
            for key in [
                EndUserCacheKey::DefaultEndUserBudget(id),
                EndUserCacheKey::TeamMemberDefaultBudget(id),
            ] {
                let _previous = values.insert(key.render(), serde_json::to_value(budget)?);
            }
        }
        if let Some(end_user) = &input.cached_end_user {
            let key = CacheKey::EndUserId(end_user.user_id.as_str()).render();
            let _previous = values.insert(key, serde_json::to_value(end_user)?);
        }
        Ok(Self {
            values: Mutex::new(values),
            effects: Mutex::new(Effects::default()),
            fail: input.fail,
            budget_row: input.budget_row.clone(),
            end_user_row: input.end_user_row.clone(),
        })
    }

    pub(super) fn tracked(&self) -> MutexGuard<'_, Effects> {
        match self.effects.lock() {
            Ok(effects) => effects,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    // the row an end-user cache write left behind, which proves a key default never reaches the cache
    pub(super) fn stored_end_user(
        &self,
        end_user_id: Option<&str>,
    ) -> Result<Value, serde_json::Error> {
        let Some(id) = end_user_id else {
            return Ok(Value::Null);
        };
        let key = CacheKey::EndUserId(id).render();
        let stored = match self.values.lock() {
            Ok(values) => values.get(&key).cloned(),
            Err(poisoned) => poisoned.into_inner().get(&key).cloned(),
        };
        let Some(stored) = stored else {
            return Ok(Value::Null);
        };
        let row: LiteLLMEndUserTable = serde_json::from_value(stored)?;
        end_user_view(&row)
    }
}

fn decode_failure() -> BudgetFetchError {
    let source = TableError::Row(serde_json::Error::io(std::io::Error::other(
        "unreadable row",
    )));
    BudgetFetchError::Decode(Box::new(AuthReadError::Repository(
        DbError::InvalidRecord { source },
    )))
}

#[async_trait]
impl BudgetRows for Fixture {
    async fn find_budget(
        &self,
        budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, BudgetFetchError> {
        let failure = match self.fail {
            Some(Failure::Decode) => Some(decode_failure()),
            Some(Failure::Query) => Some(BudgetFetchError::Query(Box::new(
                AuthReadError::Repository(DbError::NoDbConnected),
            ))),
            Some(Failure::EndUserQuery) | None => None,
        };
        self.tracked().queries.push(budget_id.to_owned());
        failure.map_or_else(|| Ok(self.budget_row.clone()), Err)
    }
}

#[async_trait]
impl EndUserRows for Fixture {
    async fn find_end_user(
        &self,
        end_user_id: &str,
    ) -> Result<Option<LiteLLMEndUserTable>, EndUserFetchError> {
        self.tracked().queries.push(end_user_id.to_owned());
        match self.fail {
            Some(Failure::Query | Failure::EndUserQuery) => Err(EndUserFetchError::Read(Box::new(
                AuthReadError::Repository(DbError::NoDbConnected),
            ))),
            Some(Failure::Decode) | None => Ok(self.end_user_row.clone()),
        }
    }
}

#[async_trait]
impl EndUserCache for Fixture {
    async fn read_row<Row>(&self, cache_key: &str) -> Result<Option<Row>, PayloadError>
    where
        Row: DeserializeOwned + Send + 'static,
    {
        self.tracked().reads.push(cache_key.to_owned());
        let stored = match self.values.lock() {
            Ok(values) => values.get(cache_key).cloned(),
            Err(poisoned) => poisoned.into_inner().get(cache_key).cloned(),
        };
        stored
            .map(serde_json::from_value)
            .transpose()
            .map_err(PayloadError::Json)
    }

    async fn write_row<Row>(
        &self,
        cache_key: &str,
        row: &Row,
        ttl: f64,
    ) -> Result<(), AuthCacheError>
    where
        Row: Serialize + Sync,
    {
        let encoded = serde_json::to_value(row)
            .map_err(|source| AuthCacheError::from(PayloadError::Json(source)))?;
        self.tracked().writes.push((cache_key.to_owned(), ttl));
        let mut values = match self.values.lock() {
            Ok(values) => values,
            Err(poisoned) => poisoned.into_inner(),
        };
        let _previous = values.insert(cache_key.to_owned(), encoded);
        Ok(())
    }
}

#[async_trait]
impl RegistryCache for Fixture {
    async fn read(&self, cache_key: &str) -> Result<Option<Json>, AuthCacheError> {
        self.tracked().reads.push(cache_key.to_owned());
        Ok(None)
    }

    async fn write(&self, cache_key: &str, _answer: &Json, ttl: f64) -> Result<(), AuthCacheError> {
        self.tracked().writes.push((cache_key.to_owned(), ttl));
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        MANAGEMENT_TTL_SECONDS
    }
}

// the money fields stay the numbers the row serialized, so the view never re-rounds a Decimal
#[derive(Serialize)]
struct BudgetView<'view> {
    budget_id: Option<&'view str>,
    max_budget: Option<Value>,
}

#[derive(Serialize)]
struct EndUserView<'view> {
    user_id: &'view str,
    budget_id: Option<&'view str>,
    spend: Option<Value>,
    max_budget: Option<Value>,
}

pub(super) fn budget_view(budget: Option<&LiteLLMBudgetTable>) -> Result<Value, serde_json::Error> {
    let Some(budget) = budget else {
        return Ok(Value::Null);
    };
    serde_json::to_value(BudgetView {
        budget_id: budget.budget_id.as_deref(),
        max_budget: serde_json::to_value(budget)?.get("max_budget").cloned(),
    })
}

pub(super) fn end_user_view(row: &LiteLLMEndUserTable) -> Result<Value, serde_json::Error> {
    let encoded = serde_json::to_value(row)?;
    let relation = encoded.get("litellm_budget_table");
    serde_json::to_value(EndUserView {
        user_id: row.user_id.as_str(),
        budget_id: row.budget_id.as_deref(),
        spend: encoded.get("spend").cloned(),
        max_budget: relation
            .and_then(|relation| relation.get("max_budget"))
            .cloned(),
    })
}
