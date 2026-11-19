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
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_cache::backend::BackendError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Value, json};
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

    fn present(&self, key: &str) -> bool {
        lock(&self.entries).contains_key(key)
    }

    fn seed<Value: Serialize>(&self, key: &str, value: &Value) {
        let payload = payload::encode(value).unwrap_or(Json::Null);
        lock(&self.entries).insert(key.to_owned(), (payload, None));
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
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

fn is_null(payload: &Json, name: &str) -> bool {
    matches!(field(payload, name), Some(Json::Null) | None)
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

    fn delete_memory(&self, _key: &str) -> Result<(), BackendError> {
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
                runtime.block_on(management(&store, team))
            }
            Input::Team {
                management_ttl,
                team_id,
                team,
                stale_alias,
            } => {
                let store = Store::new(*management_ttl);
                runtime.block_on(team_refresh(&store, team_id, team, stale_alias))
            }
            Input::Key {
                management_ttl,
                hashed_token,
                rehash_candidate,
                auth,
            } => {
                let store = Store::new(*management_ttl);
                runtime.block_on(key_refresh(&store, hashed_token, rehash_candidate, auth))
            }
            Input::Access {
                management_ttl,
                access_group_ttl,
                access_group_id,
                table,
            } => {
                let store = Store::new(*management_ttl);
                runtime.block_on(access_group(
                    &store,
                    *access_group_ttl,
                    access_group_id,
                    table,
                ))
            }
        }
    }
}

async fn management(store: &Arc<Store>, team: &JsonData) -> Result<Value, Box<HelperError>> {
    let row = LiteLLMTeamTableCachedObj::try_from(team.clone()).or_value_error()?;
    let key = format!("team_id:{}", row.team.fields.base.team_id);
    let invalidation = invalidation(store, 0.0_f64);
    invalidation
        .cache_management_object(&key, &row)
        .await
        .or_value_error()?;
    let stored = store.entry(&key);
    Ok(json!({
        "management_model_type_and_ttl": {
            "key": key,
            "ttl": stored.1,
            "team_id": text(&stored.0, "team_id"),
            "team_alias": text(&stored.0, "team_alias"),
        },
        "management_logging_argument_unused": true,
    }))
}

async fn team_refresh(
    store: &Arc<Store>,
    team_id: &str,
    team: &JsonData,
    stale_alias: &JsonData,
) -> Result<Value, Box<HelperError>> {
    let mut row = LiteLLMTeamTableCachedObj::try_from(team.clone()).or_value_error()?;
    let alias = row.team.fields.base.team_alias.clone();
    let alias_key = alias.as_ref().map(|alias| format!("team_alias:{alias}"));
    if let Some(alias_key) = alias_key.as_ref() {
        store.seed(alias_key, stale_alias);
    }
    let was_none = row.last_refreshed_at.is_none();
    let invalidation = invalidation(store, 0.0_f64);
    invalidation
        .cache_team_object(team_id, &mut row)
        .await
        .or_value_error()?;
    let key = format!("team_id:{team_id}");
    let stored = store.entry(&key);
    let alias_present = alias_key
        .as_ref()
        .is_some_and(|alias_key| store.present(alias_key));
    let stored_timestamp = number(&stored.0, "last_refreshed_at");
    Ok(json!({
        "team_id_and_alias_keys": {
            "team_key": key,
            "team_key_ttl": stored.1,
            "alias_key": alias_key,
            "alias_present": alias_present,
        },
        "team_timestamp_mutates_input": {
            "input_was_none": was_none,
            "input_now_is_none": row.last_refreshed_at.is_none(),
            "stored_has_timestamp": stored_timestamp.is_some(),
            "stored_equals_input": stored_timestamp == row.last_refreshed_at,
        },
    }))
}

async fn key_refresh(
    store: &Arc<Store>,
    hashed_token: &str,
    rehash_candidate: &str,
    auth: &BTreeMap<String, JsonData>,
) -> Result<Value, Box<HelperError>> {
    let mut original = UserAPIKeyAuth::construct(AuthInput::Dict(auth.clone())).or_value_error()?;
    let invalidation = invalidation(store, 0.0_f64);
    invalidation
        .cache_key_object(hashed_token, &mut original)
        .await
        .or_value_error()?;
    let original_wire = serde_json::to_value(&original).or_value_error()?;
    let original_timestamp = original_wire
        .get("last_refreshed_at")
        .and_then(Value::as_f64);
    let original_route = original_wire
        .get("request_route")
        .cloned()
        .unwrap_or(Value::Null);
    let stored = store.entry(hashed_token);
    let stored_timestamp = number(&stored.0, "last_refreshed_at");
    Ok(json!({
        "key_timestamp_and_request_field_copy": {
            "stored_ttl": stored.1,
            "stored_request_route_is_null": is_null(&stored.0, "request_route"),
            "stored_has_timestamp": stored_timestamp.is_some(),
        },
        "key_hash_not_recomputed": {
            "supplied_key_present": store.present(hashed_token),
            "rehash_key_present": store.present(rehash_candidate),
        },
        "key_copy_retains_original_request_fields": {
            "original_request_route": original_route,
            "original_last_refreshed_at_matches_stored": original_timestamp == stored_timestamp,
        },
    }))
}

async fn access_group(
    store: &Arc<Store>,
    access_group_ttl: f64,
    access_group_id: &str,
    table: &JsonData,
) -> Result<Value, Box<HelperError>> {
    let wire = serde_json::to_value(table).or_value_error()?;
    let row: AccessGroupTable = serde_json::from_value(wire).or_value_error()?;
    let invalidation = invalidation(store, access_group_ttl);
    invalidation
        .cache_access_object(access_group_id, &row)
        .await
        .or_value_error()?;
    let key = format!("access_group_id:{access_group_id}");
    let stored = store.entry(&key);
    Ok(json!({
        "access_group_key_and_independent_ttl": {
            "key": key,
            "ttl": stored.1,
            "access_group_id": text(&stored.0, "access_group_id"),
            "access_group_name": text(&stored.0, "access_group_name"),
        },
    }))
}
