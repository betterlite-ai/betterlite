use std::collections::BTreeMap;
use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;

use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use betterlite_redis::redis_cache_values::{RedisCacheValues, RedisCacheValuesSettings};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use betterlite_redis::redis_connection_settings::model::settings::{
    ClientKind, RedisConnectionSettings,
};
use betterlite_redis::redis_connection_settings::resolve::resolve_redis_kwargs;
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
        connection_settings(server.addr)?,
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

fn connection_settings(addr: SocketAddr) -> Result<RedisConnectionSettings, Box<HelperError>> {
    let overrides = serde_json::from_value(json!({
        "host": addr.ip().to_string(),
        "port": addr.port(),
        "socket_timeout": 2.0_f64,
        "socket_connect_timeout": 2.0_f64,
    }))
    .map_err(|error| helper_error("InvalidInput", error))?;
    resolve_redis_kwargs(overrides, ClientKind::Async, &BTreeMap::new())
        .map_err(|error| helper_error("RedisError", error))
}

type Outcome = Result<(), Box<HelperError>>;

pub(crate) async fn run_write(call: &WriteCall, values: &RedisCacheValues) -> Outcome {
    let step = write_plan(call)?;
    step.execute(values)
        .await
        .map_err(|error| helper_error("RedisError", error))
}

enum WriteStep {
    Single(Option<String>, Value, Option<f64>, bool),
    Pipeline(Vec<(String, Value)>, Option<f64>),
    PipelineTtls(Vec<(String, Value, Option<f64>)>),
    Sadd(String, Vec<String>, Option<f64>),
    Buffered(Vec<(String, Value)>),
}

impl WriteStep {
    async fn execute(&self, values: &RedisCacheValues) -> Result<(), RedisCacheValuesError> {
        match self {
            Self::Single(key, value, ttl, nx) => values
                .async_set_cache(key.as_deref(), value, *ttl, *nx)
                .await
                .map(|_stored| ()),
            Self::Pipeline(entries, ttl) => values.async_set_cache_pipeline(entries, *ttl).await,
            Self::PipelineTtls(entries) => values.async_set_cache_pipeline_with_ttls(entries).await,
            Self::Sadd(key, members, ttl) => values.async_set_cache_sadd(key, members, *ttl).await,
            Self::Buffered(entries) => {
                for (key, value) in entries {
                    values.batch_cache_write(key, value).await?;
                }
                Ok(())
            }
        }
    }
}

fn write_plan(call: &WriteCall) -> Result<WriteStep, Box<HelperError>> {
    match call.operation {
        WriteOperation::AsyncSetCache => {
            let entry = first_entry(call)?;
            Ok(WriteStep::Single(
                entry.key.clone(),
                entry.value.clone(),
                entry.ttl,
                call.nx,
            ))
        }
        WriteOperation::AsyncSetCachePipeline => Ok(WriteStep::Pipeline(
            keyed_entries(call)?,
            call.entries.first().and_then(|entry| entry.ttl),
        )),
        WriteOperation::AsyncSetCachePipelineWithTtls => {
            let mut entries = Vec::with_capacity(call.entries.len());
            for entry in &call.entries {
                entries.push((required_key(entry)?, entry.value.clone(), entry.ttl));
            }
            Ok(WriteStep::PipelineTtls(entries))
        }
        WriteOperation::AsyncSetCacheSadd => {
            let entry = first_entry(call)?;
            let members = serde_json::from_value(entry.value.clone())
                .map_err(|error| helper_error("InvalidInput", error))?;
            Ok(WriteStep::Sadd(required_key(entry)?, members, entry.ttl))
        }
        WriteOperation::BatchCacheWrite => Ok(WriteStep::Buffered(keyed_entries(call)?)),
    }
}

pub(crate) fn run_read(
    call: &ReadCall,
    values: &RedisCacheValues,
    server: &FakeServer,
) -> Result<Value, Box<HelperError>> {
    match call.operation {
        ReadOperation::GetCache => {
            let Some(key) = call.keys.first().and_then(Option::as_deref) else {
                return Err(helper_error("InvalidInput", "get_cache needs a key"));
            };
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

fn first_entry(call: &WriteCall) -> Result<&EntryInput, Box<HelperError>> {
    call.entries
        .first()
        .ok_or_else(|| helper_error("InvalidInput", "this operation needs an entry"))
}

fn keyed_entries(call: &WriteCall) -> Result<Vec<(String, Value)>, Box<HelperError>> {
    call.entries
        .iter()
        .map(|entry| Ok((required_key(entry)?, entry.value.clone())))
        .collect()
}

fn required_key(entry: &EntryInput) -> Result<String, Box<HelperError>> {
    entry
        .key
        .clone()
        .ok_or_else(|| helper_error("InvalidInput", "this operation needs a key"))
}

fn keys_sent(server: &FakeServer, name: &str) -> Vec<String> {
    let sent = server
        .commands()
        .into_iter()
        .find(|command| command.first().is_some_and(|first| first.as_str() == name));
    sent.map_or_else(Vec::new, |command| command.into_iter().skip(1).collect())
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
