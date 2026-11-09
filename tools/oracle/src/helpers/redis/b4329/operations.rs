use std::collections::BTreeMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::cache_primitives::version::RedisVersionValue;
use betterlite_redis::list_operations::error::RedisListOperationsError;
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
use betterlite_redis::redis_connection_settings::model::settings::{
    ClientKind, RedisConnectionSettings,
};
use betterlite_redis::redis_connection_settings::resolve::resolve_redis_kwargs;
use betterlite_redis::redis_live_client::LiveClientFactory;
use serde_json::{Map, Value, json};

use betterlite_redis::list_operations::results::{LpopPipelineEntry, LpopResult};

use super::fake_server::FakeServer;
use super::helper_error;
use super::input::{Call, ListValue, Operation, PipelineOperation};
use crate::helpers::HelperError;

const DEFAULT_MAJOR_VERSION: u32 = 7;

#[derive(Clone, Debug)]
pub(crate) struct RecordedEvent {
    pub(crate) call_type: String,
    pub(crate) outcome: &'static str,
}

#[derive(Default)]
pub(crate) struct RecordingHooks {
    events: Mutex<Vec<RecordedEvent>>,
}

impl RecordingHooks {
    fn record(&self, call_type: &'static str, outcome: &'static str) {
        let event = RecordedEvent {
            call_type: call_type.to_owned(),
            outcome,
        };
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

pub(crate) fn build(
    server: &FakeServer,
    call: &Call,
    hooks: Arc<RecordingHooks>,
) -> Result<RedisListOperations, Box<HelperError>> {
    let transport = RedisTransport::new(
        connection_settings(server.addr)?,
        RedisCacheFamily::Standalone,
        Arc::new(LiveClientFactory::new(None)),
        Arc::new(RedisClientStore::new()),
        None,
    )
    .map_err(|error| helper_error("RedisError", error))?;
    Ok(RedisListOperations::new(
        RedisListOperationsSettings {
            namespace: call.namespace.clone(),
            redis_version: RedisVersionValue::Text(call.redis_version.clone()),
            default_major_version: DEFAULT_MAJOR_VERSION,
        },
        Arc::new(transport),
        hooks,
        Arc::new(NoopGate),
    ))
}

pub(crate) async fn run(
    call: &Call,
    lists: &RedisListOperations,
) -> Result<Value, Box<HelperError>> {
    match call.operation {
        Operation::Rpush => {
            let key = required_key(call)?;
            let values: Vec<RedisScalar> = call.values.iter().map(ListValue::scalar).collect();
            let length = lists.async_rpush(key, &values).await?;
            Ok(json!(length))
        }
        Operation::RpushAndTrim => {
            let key = required_key(call)?;
            let max_len = call
                .max_len
                .ok_or_else(|| helper_error("InvalidInput", "rpush_and_trim needs max_len"))?;
            let values: Vec<RedisScalar> = call.values.iter().map(ListValue::scalar).collect();
            let length = lists.async_rpush_and_trim(key, &values, max_len).await?;
            Ok(json!(length))
        }
        Operation::RpushPipeline => {
            let operations: Vec<RedisPipelineRpushOperation> =
                call.operations.iter().map(rpush_operation).collect();
            let lengths = lists.async_rpush_pipeline(&operations).await?;
            Ok(json!(lengths))
        }
        Operation::Lpop => {
            let key = required_key(call)?;
            let result = lists.async_lpop(key, call.count).await?;
            Ok(lpop_result(&result))
        }
        Operation::LpopPipeline => {
            let operations: Vec<RedisPipelineLpopOperation> =
                call.operations.iter().map(lpop_operation).collect();
            let entries = lists.async_lpop_pipeline(&operations).await?;
            Ok(Value::Array(entries.iter().map(lpop_entry).collect()))
        }
    }
}

impl From<RedisListOperationsError> for Box<HelperError> {
    fn from(error: RedisListOperationsError) -> Self {
        helper_error("RedisError", error)
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

pub(crate) fn lpop_result(result: &LpopResult) -> Value {
    match result {
        LpopResult::Missing => Value::Null,
        LpopResult::Item(text) => json!(text),
        LpopResult::ItemBytes(bytes) => json!({ "bytes": bytes }),
        LpopResult::Items(items) => json!(items),
        LpopResult::ItemsBytes(items) => json!(items),
    }
}

pub(crate) fn lists(lists: &BTreeMap<String, Vec<Vec<u8>>>) -> Value {
    let mut object = Map::new();
    for (key, rows) in lists {
        object.insert(key.clone(), json!(rows));
    }
    Value::Object(object)
}

pub(crate) fn events(events: &[RecordedEvent]) -> Value {
    Value::Array(
        events
            .iter()
            .map(|event| json!({ "call_type": event.call_type, "outcome": event.outcome }))
            .collect(),
    )
}

pub(crate) fn lpop_entry(entry: &LpopPipelineEntry) -> Value {
    match entry {
        LpopPipelineEntry::Missing => Value::Null,
        LpopPipelineEntry::Items(items) => json!(items),
        LpopPipelineEntry::ItemsBytes(items) => json!(items),
    }
}
