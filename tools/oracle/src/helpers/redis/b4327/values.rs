use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;

use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache_values::{RedisCacheValues, RedisCacheValuesSettings};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use betterlite_redis::redis_connection_settings::model::settings::RedisConnectionSettings;
use betterlite_redis::redis_live_client::LiveClientFactory;
use serde_json::{Map, Value, json};

use super::fake_server::FakeServer;
use super::helper_error;
use super::input::{EntryInput, ReadCall, ReadOperation, WriteCall, WriteOperation};
use crate::helpers::HelperError;

struct NoopHooks;

impl ServiceHooks for NoopHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

struct NoopGate;

impl BreakerGate for NoopGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

pub(crate) fn build(
    server: &FakeServer,
    namespace: Option<&str>,
    default_ttl: Option<i64>,
    flush_size: Option<usize>,
) -> Result<RedisCacheValues, Box<HelperError>> {
    let transport = RedisTransport::new(
        connection_settings(server.addr),
        RedisCacheFamily::Standalone,
        Arc::new(LiveClientFactory::new(None)),
        Arc::new(RedisClientStore::new()),
        None,
    )
    .map_err(|error| helper_error("RedisError", error))?;
    Ok(RedisCacheValues::new(
        RedisCacheValuesSettings {
            namespace: namespace.map(str::to_owned),
            default_ttl,
            redis_flush_size: flush_size,
        },
        Arc::new(transport),
        Arc::new(NoopHooks),
        Arc::new(NoopGate),
    ))
}

fn connection_settings(addr: SocketAddr) -> RedisConnectionSettings {
    RedisConnectionSettings {
        host: Some(addr.ip().to_string()),
        port: Some(i64::from(addr.port())),
        db: None,
        username: None,
        password: None,
        url: None,
        ssl: None,
        socket_timeout: Some(2.0_f64),
        socket_connect_timeout: Some(2.0_f64),
        socket_keepalive: None,
        max_connections: None,
        health_check_interval: None,
        credential_provider: false,
        credential_provider_settings: None,
        startup_nodes: None,
        sentinel_nodes: None,
        sentinel_password: None,
        service_name: None,
        ssl_password: None,
        passthrough: BTreeMap::new(),
    }
}

pub(crate) async fn run_write(
    call: &WriteCall,
    values: &RedisCacheValues,
) -> Result<(), Box<HelperError>> {
    match call.operation {
        WriteOperation::AsyncSetCache => write_one(call, values).await,
        WriteOperation::AsyncSetCachePipeline => write_pipeline(call, values).await,
        WriteOperation::AsyncSetCachePipelineWithTtls => {
            write_pipeline_with_ttls(call, values).await
        }
        WriteOperation::AsyncSetCacheSadd => write_set(call, values).await,
        WriteOperation::BatchCacheWrite => write_buffered(call, values).await,
    }
}

pub(crate) fn run_read(
    call: &ReadCall,
    values: &RedisCacheValues,
    server: &FakeServer,
) -> Result<Value, Box<HelperError>> {
    match call.operation {
        ReadOperation::GetCache => {
            let key = call
                .keys
                .first()
                .and_then(Option::as_deref)
                .ok_or_else(|| helper_error("InvalidInput", "get_cache needs a key"))?;
            let cached = values
                .get_cache(key)
                .map_err(|error| helper_error("RedisError", error))?;
            Ok(json!({
                "values": cached.as_ref().map_or(Value::Null, cached_json),
                "keys_sent": keys_sent(server, "GET"),
            }))
        }
        ReadOperation::BatchGetCache => {
            let keys: Vec<Option<&str>> = call.keys.iter().map(Option::as_deref).collect();
            let cached = values
                .batch_get_cache(&keys)
                .map_err(|error| helper_error("RedisError", error))?;
            Ok(json!({
                "values": batch_json(&cached),
                "keys_sent": keys_sent(server, "MGET"),
            }))
        }
    }
}

async fn write_one(call: &WriteCall, values: &RedisCacheValues) -> Result<(), Box<HelperError>> {
    let entry = call
        .entries
        .first()
        .ok_or_else(|| helper_error("InvalidInput", "async_set_cache needs an entry"))?;
    let key = entry.key.as_deref();
    values
        .async_set_cache(key, &entry.value, entry.ttl, call.nx)
        .await
        .map_err(|error| helper_error("RedisError", error))?;
    Ok(())
}

async fn write_pipeline(
    call: &WriteCall,
    values: &RedisCacheValues,
) -> Result<(), Box<HelperError>> {
    let list = entry_pairs(call)?;
    let ttl = call.entries.first().and_then(|entry| entry.ttl);
    values
        .async_set_cache_pipeline(&list, ttl)
        .await
        .map_err(|error| helper_error("RedisError", error))
}

async fn write_pipeline_with_ttls(
    call: &WriteCall,
    values: &RedisCacheValues,
) -> Result<(), Box<HelperError>> {
    let mut list = Vec::with_capacity(call.entries.len());
    for entry in &call.entries {
        list.push((required_key(entry)?, entry.value.clone(), entry.ttl));
    }
    values
        .async_set_cache_pipeline_with_ttls(&list)
        .await
        .map_err(|error| helper_error("RedisError", error))
}

async fn write_set(call: &WriteCall, values: &RedisCacheValues) -> Result<(), Box<HelperError>> {
    let entry = call
        .entries
        .first()
        .ok_or_else(|| helper_error("InvalidInput", "async_set_cache_sadd needs an entry"))?;
    let key = required_key(entry)?;
    let members = string_array(&entry.value)?;
    values
        .async_set_cache_sadd(&key, &members, entry.ttl)
        .await
        .map_err(|error| helper_error("RedisError", error))
}

async fn write_buffered(
    call: &WriteCall,
    values: &RedisCacheValues,
) -> Result<(), Box<HelperError>> {
    for entry in &call.entries {
        let key = required_key(entry)?;
        values
            .batch_cache_write(&key, &entry.value)
            .await
            .map_err(|error| helper_error("RedisError", error))?;
    }
    Ok(())
}

fn entry_pairs(call: &WriteCall) -> Result<Vec<(String, Value)>, Box<HelperError>> {
    let mut list = Vec::with_capacity(call.entries.len());
    for entry in &call.entries {
        list.push((required_key(entry)?, entry.value.clone()));
    }
    Ok(list)
}

fn required_key(entry: &EntryInput) -> Result<String, Box<HelperError>> {
    entry
        .key
        .clone()
        .ok_or_else(|| helper_error("InvalidInput", "this operation needs a key"))
}

fn string_array(value: &Value) -> Result<Vec<String>, Box<HelperError>> {
    let Value::Array(items) = value else {
        return Err(helper_error("InvalidInput", "the members must be a list"));
    };
    items
        .iter()
        .map(|item| match item {
            Value::String(text) => Ok(text.clone()),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Array(_)
            | Value::Object(_) => Err(helper_error(
                "InvalidInput",
                "every member must be a string",
            )),
        })
        .collect()
}

fn keys_sent(server: &FakeServer, name: &str) -> Vec<String> {
    server
        .commands()
        .into_iter()
        .find(|command| command.first().is_some_and(|first| first.as_str() == name))
        .map_or_else(Vec::new, |command| command.into_iter().skip(1).collect())
}

fn cached_json(value: &CachedValue) -> Value {
    match value {
        CachedValue::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, entry)| (key.clone(), cached_json(entry)))
                .collect(),
        ),
        CachedValue::Null => Value::Null,
        CachedValue::Boolean(flag) => Value::Bool(*flag),
        CachedValue::Number(number) => Value::Number(number.clone()),
        CachedValue::Text(text) => Value::String(text.clone()),
        CachedValue::Array(items) => Value::Array(items.iter().map(cached_json).collect()),
    }
}

fn batch_json(values: &HashMap<String, Option<CachedValue>>) -> Value {
    let ordered: BTreeMap<&String, &Option<CachedValue>> = values.iter().collect();
    let mut object = Map::new();
    for (key, value) in ordered {
        object.insert(key.clone(), value.as_ref().map_or(Value::Null, cached_json));
    }
    Value::Object(object)
}
