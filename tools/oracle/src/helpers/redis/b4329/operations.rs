use std::collections::BTreeMap;
use std::error::Error;
use std::sync::{Arc, Mutex};

use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::cache_primitives::version::RedisVersionValue;
use betterlite_redis::list_operations::{
    RedisListOperations, RedisListOperationsSettings, RedisPipelineLpopOperation,
    RedisPipelineRpushOperation,
};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use betterlite_redis::redis_connection_settings::model::settings::ClientKind;
use betterlite_redis::redis_connection_settings::resolve::resolve_redis_kwargs;
use betterlite_redis::redis_live_client::LiveClientFactory;
use serde_json::{Map, Value, json};

use betterlite_redis::list_operations::results::{LpopPipelineEntry, LpopResult};

use super::fake_server::FakeServer;
use super::helper_error;
use super::input::{Call, ListValue, Operation, PipelineOperation};
use crate::helpers::HelperError;

const DEFAULT_MAJOR_VERSION: u32 = 7;

pub(crate) type RecordedEvent = (String, &'static str);

#[derive(Default)]
pub(crate) struct RecordingHooks {
    events: Mutex<Vec<RecordedEvent>>,
}

impl RecordingHooks {
    fn record(&self, call_type: &'static str, outcome: &'static str) {
        let event = (call_type.to_owned(), outcome);
        match self.events.lock() {
            Ok(mut events) => events.push(event),
            Err(poisoned) => poisoned.into_inner().push(event),
        }
    }

    pub(crate) fn events(&self) -> Vec<RecordedEvent> {
        match self.events.lock() {
            Ok(events) => events.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl ServiceHooks for RecordingHooks {
    fn success(&self, event: &ServiceEvent) {
        self.record(event.call_type, "success");
    }

    fn failure(&self, event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {
        self.record(event.call_type, "failure");
    }
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

fn connect(server: &FakeServer) -> Result<RedisTransport, Box<HelperError>> {
    let overrides = serde_json::from_value(json!({
        "host": server.addr.ip().to_string(),
        "port": server.addr.port(),
        "socket_timeout": 2.0_f64,
        "socket_connect_timeout": 2.0_f64,
    }))
    .map_err(|error| helper_error("InvalidInput", error))?;
    let settings = resolve_redis_kwargs(overrides, ClientKind::Async, &BTreeMap::new())
        .map_err(|error| helper_error("RedisError", error))?;
    RedisTransport::new(
        settings,
        RedisCacheFamily::Standalone,
        Arc::new(LiveClientFactory::new(None)),
        Arc::new(RedisClientStore::new()),
        None,
    )
    .map_err(|error| helper_error("RedisError", error))
}

pub(crate) fn build(
    server: &FakeServer,
    call: &Call,
    hooks: Arc<RecordingHooks>,
) -> Result<RedisListOperations, Box<HelperError>> {
    Ok(RedisListOperations::new(
        RedisListOperationsSettings {
            namespace: call.namespace.clone(),
            redis_version: RedisVersionValue::Text(call.redis_version.clone()),
            default_major_version: DEFAULT_MAJOR_VERSION,
        },
        Arc::new(connect(server)?),
        hooks,
        Arc::new(NoopGate),
    ))
}

pub(crate) async fn run(
    call: &Call,
    lists: &RedisListOperations,
) -> Result<Value, Box<HelperError>> {
    let redis = |error| helper_error("RedisError", error);
    match call.operation {
        Operation::Rpush => {
            let key = required_key(call)?;
            let values: Vec<RedisScalar> = call.values.iter().map(ListValue::scalar).collect();
            let reply = lists.async_rpush(key, &values).await;
            reply.map(|length| json!(length)).map_err(redis)
        }
        Operation::RpushAndTrim => {
            let key = required_key(call)?;
            let max_len = call
                .max_len
                .ok_or_else(|| helper_error("InvalidInput", "rpush_and_trim needs max_len"))?;
            let values: Vec<RedisScalar> = call.values.iter().map(ListValue::scalar).collect();
            let reply = lists.async_rpush_and_trim(key, &values, max_len).await;
            reply.map(|length| json!(length)).map_err(redis)
        }
        Operation::RpushPipeline => {
            let operations: Vec<RedisPipelineRpushOperation> =
                call.operations.iter().map(rpush_operation).collect();
            let reply = lists.async_rpush_pipeline(&operations).await;
            reply.map(|lengths| json!(lengths)).map_err(redis)
        }
        Operation::Lpop => {
            let key = required_key(call)?;
            let reply = lists.async_lpop(key, call.count).await;
            reply.map(|result| lpop_result(&result)).map_err(redis)
        }
        Operation::LpopPipeline => {
            let operations: Vec<RedisPipelineLpopOperation> =
                call.operations.iter().map(lpop_operation).collect();
            let reply = lists.async_lpop_pipeline(&operations).await;
            let listed = reply.map(|entries| entries.iter().map(lpop_entry).collect());
            listed.map(Value::Array).map_err(redis)
        }
    }
}

fn required_key(call: &Call) -> Result<&str, Box<HelperError>> {
    call.key
        .as_deref()
        .ok_or_else(|| helper_error("InvalidInput", "this operation needs a key"))
}

fn rpush_operation(input: &PipelineOperation) -> RedisPipelineRpushOperation {
    RedisPipelineRpushOperation {
        key: input.key.clone(),
        values: input.values.iter().map(ListValue::scalar).collect(),
    }
}

fn lpop_operation(input: &PipelineOperation) -> RedisPipelineLpopOperation {
    RedisPipelineLpopOperation {
        key: input.key.clone(),
        count: input.count,
    }
}

pub(crate) fn lpop_result(result: &LpopResult) -> Value {
    match result {
        LpopResult::Missing => Value::Null,
        LpopResult::Item(text) => json!(text),
        LpopResult::ItemBytes(bytes) => json!({ "bytes": bytes }),
        LpopResult::Items(items) => json!(items),
        LpopResult::ItemsBytes(items) => bytes_values(items),
    }
}

pub(crate) fn lists(lists: &BTreeMap<String, Vec<Vec<u8>>>) -> Value {
    let mut object = Map::new();
    for (key, rows) in lists {
        object.insert(key.clone(), bytes_values(rows));
    }
    Value::Object(object)
}

fn bytes_values(values: &[Vec<u8>]) -> Value {
    let mut projected = Vec::with_capacity(values.len());
    for bytes in values {
        let mut object = Map::new();
        object.insert("bytes".to_owned(), json!(bytes));
        projected.push(Value::Object(object));
    }
    Value::Array(projected)
}

pub(crate) fn events(events: &[RecordedEvent]) -> Value {
    Value::Array(
        events
            .iter()
            .map(|(call_type, outcome)| json!({ "call_type": call_type, "outcome": outcome }))
            .collect(),
    )
}

pub(crate) fn lpop_entry(entry: &LpopPipelineEntry) -> Value {
    match entry {
        LpopPipelineEntry::Missing => Value::Null,
        LpopPipelineEntry::Items(items) => json!(items),
        LpopPipelineEntry::ItemsBytes(items) => bytes_values(items),
    }
}
