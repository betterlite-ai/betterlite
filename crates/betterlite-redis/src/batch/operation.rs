use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::str;
use std::sync::{Arc, Mutex};

use num_traits::ToPrimitive as _;
use redis::{ErrorKind, ServerErrorKind, Value};
use serde_json::value::RawValue;

use super::error::{BatchError, PipelineFailure};
use super::result::{SettledHook, SettledStatus};
use super::slot::Slot;
use crate::cache_primitives::decode::{CachedValue, decode_cached_value};
use crate::cache_primitives::namespace::check_and_fix_namespace;
use crate::cache_primitives::protocol::RedisScalar;
use crate::redis_cache::RedisCache;
use crate::redis_counters::scripts::RegisteredScript;
use crate::redis_live_client::LiveClientError;

const MGET_COMMAND: &str = "MGET";
const EVALSHA_COMMAND: &str = "EVALSHA";
const INCRBYFLOAT_COMMAND: &str = "INCRBYFLOAT";

pub(crate) enum Operation {
    Mget(MgetOperation),
    Script(ScriptOperation),
    Increment(IncrementOperation),
    Set(SetOperation),
    Delete(DeleteOperation),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Settlement {
    Done,
    Retry,
}

impl Operation {
    pub(crate) fn enqueue(
        &mut self,
        cache: &RedisCache,
        pipeline: &mut redis::Pipeline,
    ) -> Result<usize, PipelineFailure> {
        let namespace = cache.namespace.as_deref();
        match self {
            Self::Mget(op) => Ok(op.enqueue(namespace, pipeline)),
            Self::Script(op) => Ok(op.enqueue(namespace, pipeline)),
            Self::Increment(op) => Ok(op.enqueue(namespace, pipeline)),
            Self::Set(op) => op.enqueue(cache, pipeline),
            Self::Delete(op) => Ok(op.enqueue(namespace, pipeline)),
        }
    }

    pub(crate) fn settle(&self, replies: Vec<Result<Value, LiveClientError>>) -> Settlement {
        match self {
            Self::Mget(op) => op.settle(replies),
            Self::Script(op) => op.settle(replies),
            Self::Increment(op) => op.settle(replies),
            Self::Set(op) => op.settle(replies),
            Self::Delete(op) => op.settle(replies),
        }
    }

    pub(crate) async fn settle_alone(&self, cache: &RedisCache) {
        match self {
            Self::Mget(op) => op.settle_alone(cache).await,
            Self::Script(op) => op.settle_alone(cache).await,
            Self::Increment(op) => op.settle_alone(cache).await,
            Self::Set(op) => op.settle_alone(cache).await,
            Self::Delete(op) => op.settle_alone(cache).await,
        }
    }

    pub(crate) fn fail(&self, error: Arc<BatchError>) {
        match self {
            Self::Mget(op) => op.slot.complete(Err(error)),
            Self::Script(op) => op.slot.complete(Err(error)),
            Self::Increment(op) => op.slot.complete(Err(error)),
            Self::Set(op) => op.slot.complete(Err(error)),
            Self::Delete(op) => op.slot.complete(Err(error)),
        }
    }

    pub(crate) fn cancel(&self) {
        match self {
            Self::Mget(op) => op.slot.cancel(),
            Self::Script(op) => op.slot.cancel(),
            Self::Increment(op) => op.slot.cancel(),
            Self::Set(op) => op.slot.cancel(),
            Self::Delete(op) => op.slot.cancel(),
        }
    }

    fn hooks(&self) -> (Vec<SettledHook>, SettledStatus) {
        match self {
            Self::Mget(op) => (op.slot.take_hooks(), op.slot.status()),
            Self::Script(op) => (op.slot.take_hooks(), op.slot.status()),
            Self::Increment(op) => (op.slot.take_hooks(), op.slot.status()),
            Self::Set(op) => (op.slot.take_hooks(), op.slot.status()),
            Self::Delete(op) => (op.slot.take_hooks(), op.slot.status()),
        }
    }

    pub(crate) async fn run_hooks(&self) {
        let (hooks, status) = self.hooks();
        for hook in hooks {
            if let Some(follow_up) = hook(status) {
                follow_up.await;
            }
        }
    }

    pub(crate) fn run_hooks_now(&self) {
        let (hooks, status) = self.hooks();
        for hook in hooks {
            // a cancelled flush cannot await a follow-up; the hook ran and its future is dropped
            let _pending = hook(status);
        }
    }
}

pub(crate) struct MgetOperation {
    pub(crate) keys: Vec<String>,
    pub(crate) misses: Arc<Mutex<BTreeSet<String>>>,
    pub(crate) slot: Arc<Slot<BTreeMap<String, Option<CachedValue>>>>,
}

impl MgetOperation {
    fn enqueue(&self, namespace: Option<&str>, pipeline: &mut redis::Pipeline) -> usize {
        let mut command = redis::cmd(MGET_COMMAND);
        for key in &self.keys {
            command.arg(namespaced_key(namespace, key));
        }
        pipeline.add_command(command);
        1
    }

    fn settle(&self, replies: Vec<Result<Value, LiveClientError>>) -> Settlement {
        let (values, failure) = split_replies(replies);
        if let Some(error) = failure {
            return reply_failure(&self.slot, error);
        }
        self.complete(self.resolve(&values));
        Settlement::Done
    }

    fn resolve(
        &self,
        replies: &[Value],
    ) -> Result<BTreeMap<String, Option<CachedValue>>, BatchError> {
        let Some(reply) = replies.first() else {
            return Err(BatchError::MissingReply {
                operation: MGET_COMMAND,
            });
        };
        let Value::Array(items) = reply else {
            return Err(BatchError::MgetReply {
                kind: kind_name(reply),
            });
        };
        let mut values = BTreeMap::new();
        for (key, item) in self.keys.iter().zip(items) {
            let value = match item {
                Value::BulkString(bytes) => decode_cached_value(Some(bytes))?,
                Value::Nil => None,
                Value::Int(_)
                | Value::Array(_)
                | Value::SimpleString(_)
                | Value::Okay
                | Value::Map(_)
                | Value::Attribute { .. }
                | Value::Set(_)
                | Value::Double(_)
                | Value::Boolean(_)
                | Value::VerbatimString { .. }
                | Value::BigNumber(_)
                | Value::Push { .. }
                | Value::ServerError(_)
                | _ => {
                    return Err(BatchError::MgetValue {
                        kind: kind_name(item),
                    });
                }
            };
            values.insert(key.clone(), value);
        }
        Ok(values)
    }

    async fn settle_alone(&self, cache: &RedisCache) {
        let requested: Vec<Option<&str>> = self.keys.iter().map(|key| Some(key.as_str())).collect();
        let result = match cache.async_batch_get_cache(&requested).await {
            Ok(found) => {
                if self.keys.iter().any(|key| !found.contains_key(key)) {
                    Err(BatchError::MissingKeys {
                        requested: self.keys.len(),
                        returned: found.len(),
                    })
                } else {
                    Ok(ordered_values(&self.keys, found))
                }
            }
            Err(source) => Err(BatchError::Fallback(source)),
        };
        self.complete(result);
    }

    fn complete(&self, result: Result<BTreeMap<String, Option<CachedValue>>, BatchError>) {
        if let Ok(values) = &result {
            let mut misses = match self.misses.lock() {
                Ok(misses) => misses,
                Err(poisoned) => poisoned.into_inner(),
            };
            misses.extend(
                values
                    .iter()
                    .filter(|(_key, value)| value.is_none())
                    .map(|(key, _value)| key.clone()),
            );
        }
        self.slot.complete(result.map_err(Arc::new));
    }
}

pub(crate) struct ScriptOperation {
    pub(crate) script: Arc<RegisteredScript>,
    pub(crate) keys: Vec<String>,
    pub(crate) args: Vec<RedisScalar>,
    pub(crate) slot: Arc<Slot<Value>>,
}

impl ScriptOperation {
    fn enqueue(&self, namespace: Option<&str>, pipeline: &mut redis::Pipeline) -> usize {
        let mut command = redis::cmd(EVALSHA_COMMAND);
        command.arg(&self.script.sha1).arg(self.keys.len());
        for key in &self.keys {
            command.arg(namespaced_key(namespace, key));
        }
        for arg in &self.args {
            match arg {
                RedisScalar::Text(text) => {
                    command.arg(text);
                }
                RedisScalar::Bytes(bytes) => {
                    command.arg(bytes);
                }
                RedisScalar::Float(value) => {
                    command.arg(*value);
                }
            }
        }
        pipeline.add_command(command);
        1
    }

    fn settle(&self, replies: Vec<Result<Value, LiveClientError>>) -> Settlement {
        let (values, failure) = split_replies(replies);
        if let Some(error) = failure {
            return reply_failure(&self.slot, error);
        }
        let result = values.first().cloned().ok_or(BatchError::MissingReply {
            operation: EVALSHA_COMMAND,
        });
        self.slot.complete(result.map_err(Arc::new));
        Settlement::Done
    }

    async fn settle_alone(&self, _cache: &RedisCache) {
        let result = self
            .script
            .run(&self.keys, &self.args)
            .await
            .map_err(BatchError::Script);
        self.slot.complete(result.map_err(Arc::new));
    }
}

pub(crate) struct IncrementOperation {
    pub(crate) key: String,
    pub(crate) value: f64,
    pub(crate) ttl: Option<i64>,
    pub(crate) slot: Arc<Slot<f64>>,
}

impl IncrementOperation {
    fn enqueue(&self, namespace: Option<&str>, pipeline: &mut redis::Pipeline) -> usize {
        let name = namespaced_key(namespace, &self.key);
        let mut command = redis::cmd(INCRBYFLOAT_COMMAND);
        command.arg(&name).arg(self.value);
        pipeline.add_command(command);
        let Some(ttl) = self.ttl else {
            return 1;
        };
        let mut expire = redis::cmd("EXPIRE");
        expire.arg(&name).arg(ttl);
        pipeline.add_command(expire);
        2
    }

    fn settle(&self, replies: Vec<Result<Value, LiveClientError>>) -> Settlement {
        let (values, failure) = split_replies(replies);
        if let Some(error) = failure {
            return reply_failure(&self.slot, error);
        }
        let result = values.first().map_or_else(
            || {
                Err(BatchError::MissingReply {
                    operation: INCRBYFLOAT_COMMAND,
                })
            },
            numeric_reply,
        );
        self.slot.complete(result.map_err(Arc::new));
        Settlement::Done
    }

    async fn settle_alone(&self, cache: &RedisCache) {
        let result = cache
            .async_increment(&self.key, self.value, self.ttl, false)
            .await
            .map_err(BatchError::Fallback);
        self.slot.complete(result.map_err(Arc::new));
    }
}

pub(crate) struct SetOperation {
    pub(crate) key: String,
    pub(crate) value: String,
    pub(crate) ttl: Option<f64>,
    pub(crate) encode_error: Option<serde_json::Error>,
    pub(crate) slot: Arc<Slot<()>>,
}

impl SetOperation {
    fn enqueue(
        &mut self,
        cache: &RedisCache,
        pipeline: &mut redis::Pipeline,
    ) -> Result<usize, PipelineFailure> {
        if let Some(source) = self.encode_error.take() {
            return Err(PipelineFailure::Encode { source });
        }
        let seconds = match self.ttl {
            None => cache.default_ttl,
            Some(seconds) if seconds.is_nan() => cache.default_ttl,
            Some(seconds) => seconds
                .trunc()
                .to_i64()
                .ok_or(PipelineFailure::Ttl { seconds })?,
        };
        let mut command = redis::cmd("SET");
        command
            .arg(namespaced_key(cache.namespace.as_deref(), &self.key))
            .arg(&self.value)
            .arg("EX")
            .arg(seconds);
        pipeline.add_command(command);
        Ok(1)
    }

    fn settle(&self, replies: Vec<Result<Value, LiveClientError>>) -> Settlement {
        let (_values, failure) = split_replies(replies);
        if let Some(error) = failure {
            return reply_failure(&self.slot, error);
        }
        self.slot.complete(Ok(()));
        Settlement::Done
    }

    async fn settle_alone(&self, cache: &RedisCache) {
        let result = match RawValue::from_string(self.value.clone()) {
            Ok(raw) => cache
                .values
                .async_set_cache_pipeline_with_ttls(&[(self.key.clone(), raw, self.ttl)])
                .await
                .map_err(BatchError::SetFallback),
            Err(source) => Err(BatchError::Serialize { source }),
        };
        self.slot.complete(result.map_err(Arc::new));
    }
}

pub(crate) struct DeleteOperation {
    pub(crate) key: String,
    pub(crate) slot: Arc<Slot<()>>,
}

impl DeleteOperation {
    fn enqueue(&self, namespace: Option<&str>, pipeline: &mut redis::Pipeline) -> usize {
        let mut command = redis::cmd("DEL");
        command.arg(namespaced_key(namespace, &self.key));
        pipeline.add_command(command);
        1
    }

    fn settle(&self, replies: Vec<Result<Value, LiveClientError>>) -> Settlement {
        let (_values, failure) = split_replies(replies);
        if let Some(error) = failure {
            return reply_failure(&self.slot, error);
        }
        self.slot.complete(Ok(()));
        Settlement::Done
    }

    async fn settle_alone(&self, cache: &RedisCache) {
        let result = cache
            .admin
            .async_delete_cache(&self.key)
            .await
            .map(|_deleted| ())
            .map_err(BatchError::Delete);
        self.slot.complete(result.map_err(Arc::new));
    }
}

fn namespaced_key(namespace: Option<&str>, key: &str) -> String {
    let Some(namespaced) = check_and_fix_namespace(namespace, Some(key)) else {
        return key.to_owned();
    };
    namespaced
}

fn split_replies(
    replies: Vec<Result<Value, LiveClientError>>,
) -> (Vec<Value>, Option<LiveClientError>) {
    let mut values = Vec::with_capacity(replies.len());
    let mut failure = None;
    for reply in replies {
        match reply {
            Ok(value) => values.push(value),
            Err(error) => {
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
    }
    (values, failure)
}

fn reply_failure<T>(slot: &Slot<T>, failure: LiveClientError) -> Settlement {
    if missing_script(&failure) {
        return Settlement::Retry;
    }
    slot.complete(Err(Arc::new(BatchError::Reply(failure))));
    Settlement::Done
}

fn missing_script(failure: &LiveClientError) -> bool {
    matches!(
        failure,
        LiveClientError::Redis(error)
            if error.kind() == ErrorKind::Server(ServerErrorKind::NoScript)
    )
}

fn numeric_reply(reply: &Value) -> Result<f64, BatchError> {
    match reply {
        Value::BulkString(bytes) => {
            let text = str::from_utf8(bytes).map_err(|source| BatchError::Utf8 { source })?;
            text.parse()
                .map_err(|source| BatchError::IncrementText { source })
        }
        Value::SimpleString(text) => text
            .parse()
            .map_err(|source| BatchError::IncrementText { source }),
        Value::Int(integer) => integer
            .to_f64()
            .ok_or(BatchError::IncrementReply { kind: "an integer" }),
        Value::Double(value) => Ok(*value),
        Value::Nil
        | Value::Array(_)
        | Value::Okay
        | Value::Map(_)
        | Value::Attribute { .. }
        | Value::Set(_)
        | Value::Boolean(_)
        | Value::VerbatimString { .. }
        | Value::BigNumber(_)
        | Value::Push { .. }
        | Value::ServerError(_)
        | _ => Err(BatchError::IncrementReply {
            kind: kind_name(reply),
        }),
    }
}

fn ordered_values(
    keys: &[String],
    mut found: HashMap<String, Option<CachedValue>>,
) -> BTreeMap<String, Option<CachedValue>> {
    let mut values = BTreeMap::new();
    for key in keys {
        let value = found.remove(key).unwrap_or(None);
        values.insert(key.clone(), value);
    }
    values
}

const fn kind_name(value: &Value) -> &'static str {
    match value {
        Value::Nil => "nil",
        Value::Int(_) => "an integer",
        Value::BulkString(_) => "a bulk string",
        Value::Array(_) => "an array",
        Value::SimpleString(_) => "a simple string",
        Value::Okay => "ok",
        Value::Map(_) => "a map",
        Value::Attribute { .. } => "an attribute",
        Value::Set(_) => "a set",
        Value::Double(_) => "a double",
        Value::Boolean(_) => "a boolean",
        Value::VerbatimString { .. } => "a verbatim string",
        Value::BigNumber(_) => "a big number",
        Value::Push { .. } => "a push",
        Value::ServerError(_) => "an error",
        _ => "an unknown reply",
    }
}
