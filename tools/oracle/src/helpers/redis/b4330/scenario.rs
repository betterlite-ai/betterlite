use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_redis::batch::error::BatchError;
use betterlite_redis::batch::result::{BatchResult, Settled, SettledHook};
use betterlite_redis::batch::{MgetValues, RedisBatch};
use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_cache::RedisCache;
use redis::Value;
use serde_json::{Value as Json, json};
use tokio_util::task::TaskTracker;

use super::failure;
use super::input::{Arg, Call, Operation, Reply, Step};
use super::server::{FakeServer, connect_cache};
use super::taxonomy;
use crate::helpers::{HelperError, HelperResult};

const REPORTED: [&str; 8] = [
    "MGET",
    "EVALSHA",
    "SCRIPT",
    "INCRBYFLOAT",
    "EXPIRE",
    "SET",
    "DEL",
    "TTL",
];

enum Handle {
    Mget(BatchResult<MgetValues>),
    Increment(BatchResult<f64>),
    Set(BatchResult<()>),
    Script(BatchResult<Value>),
}

struct Walk<'input> {
    batch: &'input Arc<RedisBatch>,
    cache: &'input Arc<RedisCache>,
    log: &'input Arc<Mutex<Vec<Json>>>,
    handles: Vec<Option<Handle>>,
    results: Vec<Json>,
    misses: Vec<Json>,
}

impl Walk<'_> {
    async fn step(&mut self, step: &Step) -> Result<(), Box<HelperError>> {
        match step {
            Step::Declare(operation) => {
                let index = self.handles.len();
                let handle = declare(
                    self.batch,
                    self.cache,
                    operation,
                    index,
                    Arc::clone(self.log),
                );
                self.handles.push(Some(handle));
            }
            Step::Await(index) => {
                let Some(handle) = self.handles.get_mut(*index).and_then(Option::take) else {
                    return Err(failure(format!("no unsettled operation at {index}")));
                };
                self.results.push(take(handle, *index).await);
            }
            Step::AwaitAll => {
                for index in 0..self.handles.len() {
                    if let Some(handle) = self.handles.get_mut(index).and_then(Option::take) {
                        self.results.push(take(handle, index).await);
                    }
                }
            }
            Step::Flush => self.batch.flush().await,
            Step::Missing(key) => self
                .misses
                .push(json!({"key": key, "missing": self.batch.read_as_missing(key)})),
        }
        Ok(())
    }
}

pub(crate) async fn run(call: &Call) -> HelperResult {
    let server = FakeServer::start().map_err(failure)?;
    script_replies(&server, &call.replies);
    let target = if call.refused { None } else { Some(&server) };
    let cache = connect_cache(
        target,
        call.namespace.as_deref(),
        call.default_ttl,
        call.cluster,
    )
    .await
    .map_err(failure)?;
    let batch = Arc::new(RedisBatch::new(
        Arc::clone(&cache),
        "redis_batch",
        TaskTracker::new(),
    ));
    let log: Arc<Mutex<Vec<Json>>> = Arc::new(Mutex::new(Vec::new()));
    let mut walk = Walk {
        batch: &batch,
        cache: &cache,
        log: &log,
        handles: Vec::new(),
        results: Vec::new(),
        misses: Vec::new(),
    };
    for step in &call.steps {
        walk.step(step).await?;
    }
    batch.flush().await;
    Ok(report(&server, &batch, &log, &walk.results, &walk.misses))
}

fn script_replies(server: &FakeServer, replies: &BTreeMap<String, Vec<Reply>>) {
    for (name, scripted) in replies {
        for reply in scripted {
            server.script(name, reply.clone());
        }
    }
}

fn report(
    server: &FakeServer,
    batch: &RedisBatch,
    log: &Arc<Mutex<Vec<Json>>>,
    results: &[Json],
    misses: &[Json],
) -> Json {
    let commands: Vec<Vec<String>> = server
        .commands()
        .into_iter()
        .filter(|command| {
            command
                .first()
                .is_some_and(|name| REPORTED.contains(&name.as_str()))
        })
        .collect();
    let settled = match log.lock() {
        Ok(entries) => entries.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    json!({
        "results": results,
        "settled": settled,
        "misses": misses,
        "commands": commands,
        "flushes": batch.flushes(),
        "pending": batch.pending(),
    })
}

fn declare(
    batch: &Arc<RedisBatch>,
    cache: &Arc<RedisCache>,
    operation: &Operation,
    index: usize,
    log: Arc<Mutex<Vec<Json>>>,
) -> Handle {
    match operation {
        Operation::Mget { keys } => {
            let result = batch.mget(keys);
            result.on_settled(settled_hook(index, log));
            Handle::Mget(result)
        }
        Operation::Increment { key, value, ttl } => {
            let result = batch.increment(key, *value, *ttl);
            result.on_settled(settled_hook(index, log));
            Handle::Increment(result)
        }
        Operation::Set { key, value, ttl } => {
            let result = batch.set(key, value, *ttl);
            result.on_settled(settled_hook(index, log));
            Handle::Set(result)
        }
        Operation::Delete { key } => {
            let result = batch.delete(key);
            result.on_settled(settled_hook(index, log));
            Handle::Set(result)
        }
        Operation::Script { source, keys, args } => {
            let script = Arc::new(cache.counters.async_register_script(source));
            let scalars: Vec<RedisScalar> = args.iter().map(scalar).collect();
            let result = batch.script(&script, keys, &scalars);
            result.on_settled(settled_hook(index, log));
            Handle::Script(result)
        }
    }
}

fn settled_hook<T>(index: usize, log: Arc<Mutex<Vec<Json>>>) -> SettledHook<T>
where
    T: 'static,
{
    Arc::new(move |settled: &Settled<T>| {
        let text = match settled {
            Ok(_) => "completed",
            Err(error) if matches!(**error, BatchError::Cancelled) => "cancelled",
            Err(_) => "failed",
        };
        let entry = json!({"handle": index, "status": text});
        match log.lock() {
            Ok(mut entries) => entries.push(entry),
            Err(poisoned) => poisoned.into_inner().push(entry),
        }
        None
    })
}

fn scalar(arg: &Arg) -> RedisScalar {
    match arg {
        Arg::Text(text) => RedisScalar::Text(text.clone()),
        Arg::Int(value) => RedisScalar::Text(value.to_string()),
        Arg::Float(value) => RedisScalar::Float(*value),
    }
}

async fn take(handle: Handle, index: usize) -> Json {
    match handle {
        Handle::Mget(result) => match result.await {
            Ok(values) => json!({"handle": index, "ok": true, "value": values_json(&values)}),
            Err(error) => {
                json!({"handle": index, "ok": false, "error": taxonomy::classify(&error)})
            }
        },
        Handle::Increment(result) => match result.await {
            Ok(value) => json!({"handle": index, "ok": true, "value": value}),
            Err(error) => {
                json!({"handle": index, "ok": false, "error": taxonomy::classify(&error)})
            }
        },
        Handle::Set(result) => match result.await {
            Ok(()) => json!({"handle": index, "ok": true, "value": Json::Null}),
            Err(error) => {
                json!({"handle": index, "ok": false, "error": taxonomy::classify(&error)})
            }
        },
        Handle::Script(result) => match result.await {
            Ok(value) => json!({"handle": index, "ok": true, "value": value_json(&value)}),
            Err(error) => {
                json!({"handle": index, "ok": false, "error": taxonomy::classify(&error)})
            }
        },
    }
}

fn values_json(values: &BTreeMap<String, Option<CachedValue>>) -> Json {
    values
        .iter()
        .map(|(key, stored)| (key.clone(), stored.as_ref().map_or(Json::Null, cached_json)))
        .collect()
}

fn cached_json(stored: &CachedValue) -> Json {
    match stored {
        CachedValue::Object(entries) => entries
            .iter()
            .map(|(key, nested)| (key.clone(), cached_json(nested)))
            .collect(),
        CachedValue::Array(items) => items.iter().map(cached_json).collect(),
        CachedValue::Null => Json::Null,
        CachedValue::Boolean(flag) => json!(flag),
        CachedValue::Number(number) => Json::Number(number.clone()),
        CachedValue::Text(text) => json!(text),
    }
}

fn value_json(reply: &Value) -> Json {
    if let Value::Array(items) = reply {
        return items.iter().map(value_json).collect();
    }
    if let Value::BulkString(bytes) = reply {
        return String::from_utf8(bytes.clone())
            .map_or_else(|_error| json!(bytes), |text| json!(text));
    }
    match reply {
        Value::Int(integer) => json!(integer),
        Value::SimpleString(text) => json!(text),
        Value::Double(number) => json!(number),
        Value::Boolean(flag) => json!(flag),
        Value::Nil
        | Value::Okay
        | Value::Map(_)
        | Value::Attribute { .. }
        | Value::Set(_)
        | Value::VerbatimString { .. }
        | Value::BigNumber(_)
        | Value::Push { .. }
        | Value::ServerError(_)
        | _ => Json::Null,
    }
}
