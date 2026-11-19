mod scenarios;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_cache::payload;
use betterlite_auth::auth_invalidation::AuthInvalidation;
use betterlite_auth::auth_invalidation::clock::InvalidationClock;
use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_auth::auth_invalidation::ports::{
    AuthInvalidationCache, DeferredDelete, RedisMarker, SpendCounterCache,
};
use betterlite_auth::auth_invalidation::transport::InvalidationTransport;
use betterlite_auth::auth_pubsub::eviction::LocalEviction;
use betterlite_cache::backend::BackendError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;
use tokio::runtime::Builder;

const FIXED_NOW_MILLIS: i64 = 1_000_500;
// LiteLLM's DEFAULT_MANAGEMENT_OBJECT_IN_MEMORY_CACHE_TTL, which its cache subprocess keeps unset
const DEFAULT_MANAGEMENT_TTL: f64 = 60.0;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "scenario")]
pub(crate) enum Input {
    Management {
        management_ttl: Option<f64>,
        team: JsonData,
    },
    Team {
        management_ttl: Option<f64>,
        team_id: String,
        team: JsonData,
        stale_alias: JsonData,
    },
    Key {
        management_ttl: Option<f64>,
        hashed_token: String,
        rehash_candidate: String,
        auth: BTreeMap<String, JsonData>,
    },
    Access {
        management_ttl: Option<f64>,
        access_group_ttl: f64,
        access_group_id: String,
        table: JsonData,
    },
}

type Stored = (Json, Option<f64>);

struct Store {
    management_ttl: f64,
    entries: Mutex<BTreeMap<String, Stored>>,
}

impl Store {
    fn new(management_ttl: Option<f64>) -> Arc<Self> {
        Arc::new(Self {
            management_ttl: management_ttl.unwrap_or(DEFAULT_MANAGEMENT_TTL),
            entries: Mutex::new(BTreeMap::new()),
        })
    }

    fn entry(&self, key: &str) -> Stored {
        lock(&self.entries)
            .get(key)
            .cloned()
            .unwrap_or((Json::Null, None))
    }

    fn seed<Value: Serialize>(&self, key: &str, value: &Value) {
        lock(&self.entries).insert(
            key.to_owned(),
            (payload::encode(value).unwrap_or(Json::Null), None),
        );
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn field<'payload>(payload: &'payload Json, name: &str) -> Option<&'payload Json> {
    match payload {
        Json::Object(fields) => fields.get(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn number(payload: &Json, name: &str) -> Option<f64> {
    match field(payload, name) {
        Some(Json::Number(value)) => value.as_f64(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn text(payload: &Json, name: &str) -> Option<String> {
    match field(payload, name) {
        Some(Json::Text(value)) => Some(value.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

#[async_trait]
impl AuthInvalidationCache for Store {
    fn redis_handle(&self, _key: &str) -> Option<Arc<dyn RedisMarker>> {
        None
    }

    fn management_ttl(&self) -> f64 {
        self.management_ttl
    }

    async fn write_management(
        &self,
        key: &str,
        payload: &Json,
        ttl: Option<f64>,
    ) -> Result<(), InvalidationError> {
        lock(&self.entries).insert(key.to_owned(), (payload.clone(), ttl));
        Ok(())
    }

    fn delete_sync(&self, key: &str) -> Result<(), InvalidationError> {
        lock(&self.entries).remove(key);
        Ok(())
    }

    async fn delete_async(&self, key: &str) -> Result<(), InvalidationError> {
        lock(&self.entries).remove(key);
        Ok(())
    }

    async fn delete_pre_call(&self, key: &str) -> Result<DeferredDelete, InvalidationError> {
        lock(&self.entries).remove(key);
        Ok(DeferredDelete::Settled)
    }

    fn delete_memory(&self, key: &str) -> Result<(), BackendError> {
        lock(&self.entries).remove(key);
        Ok(())
    }

    fn delete_redis(&self, _key: &str) -> Result<(), RedisAdminError> {
        Ok(())
    }
}

#[async_trait]
impl SpendCounterCache for Store {
    fn redis_handle(&self) -> Option<Arc<dyn RedisMarker>> {
        None
    }

    fn set_memory(&self, _key: &str, _value: f64, _ttl: f64) -> Result<(), BackendError> {
        Ok(())
    }

    async fn set_redis(
        &self,
        _key: &str,
        _value: f64,
        _ttl: f64,
    ) -> Result<(), RedisCacheValuesError> {
        Ok(())
    }

    async fn delete_redis(&self, _key: &str) -> Result<(), RedisAdminError> {
        Ok(())
    }
}

struct NoTransport;

#[async_trait]
impl InvalidationTransport for NoTransport {
    async fn publish(&self, _cache_key: &str, _new_value: Option<f64>, _ttl: Option<f64>) {}

    async fn evict_and_broadcast(&self, _cache_keys: &[String], _local: &dyn LocalEviction) {}
}

struct FixedClock;

impl InvalidationClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(FIXED_NOW_MILLIS).unwrap_or(DateTime::UNIX_EPOCH)
    }
}

fn invalidation(store: &Arc<Store>, access_group_ttl: f64) -> AuthInvalidation {
    let auth: Arc<dyn AuthInvalidationCache> = Arc::clone(store);
    let spend: Arc<dyn SpendCounterCache> = Arc::clone(store);
    AuthInvalidation {
        auth,
        usage: None,
        spend,
        transport: Arc::new(NoTransport),
        clock: Arc::new(FixedClock),
        access_group_ttl,
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        match input {
            Input::Management {
                management_ttl,
                team,
            } => {
                let store = Store::new(*management_ttl);
                runtime.block_on(scenarios::management(&store, team))
            }
            Input::Team {
                management_ttl,
                team_id,
                team,
                stale_alias,
            } => {
                let store = Store::new(*management_ttl);
                runtime.block_on(scenarios::team_refresh(&store, team_id, team, stale_alias))
            }
            Input::Key {
                management_ttl,
                hashed_token,
                rehash_candidate,
                auth,
            } => {
                let store = Store::new(*management_ttl);
                runtime.block_on(scenarios::key_refresh(
                    &store,
                    hashed_token,
                    rehash_candidate,
                    auth,
                ))
            }
            Input::Access {
                management_ttl,
                access_group_ttl,
                access_group_id,
                table,
            } => {
                let store = Store::new(*management_ttl);
                runtime.block_on(scenarios::access_group(
                    &store,
                    *access_group_ttl,
                    access_group_id,
                    table,
                ))
            }
        }
    }
}
