use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use redis::Value;

use super::error::BatchError;
use super::operation::Settlement;
use crate::cache_primitives::decode::CachedValue;
use crate::cache_primitives::namespace::check_and_fix_namespace;
use crate::redis_live_client::LiveClientError;

pub(crate) fn namespaced(namespace: Option<&str>, key: &str) -> String {
    let Some(prefixed) = check_and_fix_namespace(namespace, Some(key)) else {
        return key.to_owned();
    };
    prefixed
}

fn split(replies: Vec<Result<Value, LiveClientError>>) -> (Vec<Value>, Option<LiveClientError>) {
    let mut values = Vec::with_capacity(replies.len());
    let mut failed = None;
    for reply in replies {
        match reply {
            Ok(value) => values.push(value),
            Err(error) => {
                if failed.is_none() {
                    failed = Some(error);
                }
            }
        }
    }
    (values, failed)
}

fn missing_script(failure: &LiveClientError) -> bool {
    matches!(
        failure,
        LiveClientError::Redis(error)
            if error.kind() == redis::ErrorKind::Server(redis::ServerErrorKind::NoScript)
    )
}

pub(crate) fn settle_pipeline<T, Resolve, Complete>(
    replies: Vec<Result<Value, LiveClientError>>,
    resolve: Resolve,
    complete: Complete,
) -> Settlement
where
    Resolve: FnOnce(&[Value]) -> Result<T, BatchError>,
    Complete: FnOnce(Result<T, Arc<BatchError>>),
{
    let (values, failed) = split(replies);
    if let Some(error) = failed {
        if missing_script(&error) {
            return Settlement::Retry;
        }
        complete(Err(Arc::new(BatchError::Reply(error))));
        return Settlement::Done;
    }
    complete(resolve(&values).map_err(Arc::new));
    Settlement::Done
}

pub(crate) const fn kind(value: &Value) -> &'static str {
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

pub(crate) fn ordered(
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
