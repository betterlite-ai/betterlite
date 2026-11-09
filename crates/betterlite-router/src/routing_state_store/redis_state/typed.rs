use std::collections::BTreeSet;

use betterlite_redis::cache_primitives::protocol::AsyncRedisCommands as _;
use redis::cmd;

use super::super::error::{Observed, StateStoreError};
use super::codec::{decode, encode, reply_bytes, set_command};
use super::{RedisState, guarded, namespaced, observe, resolve_float_ttl, typed_ttl};
use crate::routing_value_store::{
    CompareOutcome, RouteIncrement, RouteKey, RouteValue, StoreError, StoreRead,
};

const UPDATE_ATTEMPTS: usize = 8;

pub(crate) async fn get(
    state: &RedisState,
    key: &RouteKey,
) -> Result<Observed<StoreRead>, StateStoreError> {
    observe(state, "async_get_cache", StoreRead::Missing, || {
        read_one(state, key)
    })
    .await
}

pub(crate) async fn batch_get(
    state: &RedisState,
    keys: &[RouteKey],
) -> Result<Observed<Vec<StoreRead>>, StateStoreError> {
    let fallback = vec![StoreRead::Missing; keys.len()];
    observe(state, "async_batch_get_cache", fallback, || {
        read_batch(state, keys)
    })
    .await
}

pub(crate) async fn set(
    state: &RedisState,
    key: &RouteKey,
    value: &RouteValue,
    ttl_seconds: Option<f64>,
) -> Result<Observed<()>, StateStoreError> {
    let Some(bytes) = encode(key.as_ref(), value)? else {
        return Ok(Observed {
            value: (),
            warning: None,
        });
    };
    let ttl = typed_ttl(ttl_seconds)?;
    let namespaced = namespaced(state, key);
    guarded(state, "async_set_cache", move || {
        write_one(state, namespaced, bytes, ttl)
    })
    .await
}

pub(crate) async fn set_pipeline(
    state: &RedisState,
    entries: &[(RouteKey, RouteValue)],
    ttl_seconds: Option<f64>,
) -> Result<Observed<()>, StateStoreError> {
    let mut commands = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        if let Some(bytes) = encode(key.as_ref(), value)? {
            commands.push((namespaced(state, key), bytes));
        }
    }
    if commands.is_empty() {
        return Ok(Observed {
            value: (),
            warning: None,
        });
    }
    let ttl = typed_ttl(ttl_seconds)?;
    guarded(state, "async_set_cache_pipeline", move || {
        write_many(state, commands, ttl)
    })
    .await
}

pub(crate) async fn set_add(
    state: &RedisState,
    key: &RouteKey,
    values: &[String],
    ttl_seconds: Option<f64>,
) -> Result<Observed<Vec<String>>, StateStoreError> {
    let ttl = typed_ttl(ttl_seconds)?;
    let namespaced = namespaced(state, key);
    guarded(state, "set_add", move || {
        add_members(state, key, namespaced, values.to_vec(), ttl)
    })
    .await
}

pub(crate) async fn delete(
    state: &RedisState,
    key: &RouteKey,
) -> Result<Observed<()>, StateStoreError> {
    let namespaced = namespaced(state, key);
    guarded(state, "async_delete_cache", move || {
        delete_one(state, namespaced)
    })
    .await
}

pub(crate) async fn get_ttl(
    state: &RedisState,
    key: &RouteKey,
) -> Result<Observed<Option<i64>>, StateStoreError> {
    let namespaced = namespaced(state, key);
    observe(state, "async_get_ttl", None, move || async move {
        let client = state.transport.async_client().await?;
        let ttl = client
            .ttl(&namespaced)
            .await
            .map_err(|source| StateStoreError::Redis {
                operation: "async_get_ttl",
                source,
            })?;
        Ok((ttl > -1).then_some(ttl))
    })
    .await
}

pub(crate) async fn refresh_ttl(
    state: &RedisState,
    key: &RouteKey,
    ttl_seconds: Option<f64>,
) -> Result<Observed<bool>, StateStoreError> {
    let ttl = resolve_float_ttl(state, ttl_seconds);
    let namespaced = namespaced(state, key);
    guarded(state, "async_refresh_ttl", move || {
        expire_one(state, namespaced, ttl)
    })
    .await
}

pub(crate) async fn increment(
    state: &RedisState,
    key: &RouteKey,
    by: &RouteIncrement,
    ttl_seconds: Option<f64>,
) -> Result<Observed<RouteValue>, StateStoreError> {
    let ttl = typed_ttl(ttl_seconds)?;
    let namespaced = namespaced(state, key);
    let key_name = key.as_ref().to_owned();
    guarded(state, "async_increment", move || {
        increment_until_written(state, namespaced, key_name, by, ttl)
    })
    .await
}

pub(crate) async fn compare_and_set(
    state: &RedisState,
    key: &RouteKey,
    expected: &StoreRead,
    value: &RouteValue,
    ttl_seconds: Option<f64>,
) -> Result<Observed<CompareOutcome>, StateStoreError> {
    let ttl = typed_ttl(ttl_seconds)?;
    let namespaced = namespaced(state, key);
    guarded(state, "compare_and_set", move || async move {
        super::cas::compare_and_set(state, &namespaced, expected, value, ttl).await
    })
    .await
}

async fn read_one(state: &RedisState, key: &RouteKey) -> Result<StoreRead, StateStoreError> {
    let namespaced = namespaced(state, key);
    let client = state.transport.async_client().await?;
    let mut command = cmd("GET");
    command.arg(&namespaced);
    let reply: Option<Vec<u8>> =
        client
            .query(&command)
            .await
            .map_err(|source| StateStoreError::Redis {
                operation: "async_get_cache",
                source,
            })?;
    decode(reply)
}

async fn read_batch(
    state: &RedisState,
    keys: &[RouteKey],
) -> Result<Vec<StoreRead>, StateStoreError> {
    let namespaced: Vec<String> = keys.iter().map(|key| namespaced(state, key)).collect();
    let replies = state.transport.async_mget(&namespaced).await?;
    let mut values = Vec::with_capacity(replies.len());
    for reply in replies {
        values.push(decode(reply_bytes("async_batch_get_cache", reply)?)?);
    }
    Ok(values)
}

async fn write_one(
    state: &RedisState,
    key: String,
    bytes: Vec<u8>,
    ttl: Option<i64>,
) -> Result<(), StateStoreError> {
    let client = state.transport.async_client().await?;
    client
        .query::<()>(&set_command(&key, &bytes, ttl))
        .await
        .map_err(|source| StateStoreError::Redis {
            operation: "async_set_cache",
            source,
        })
}

async fn write_many(
    state: &RedisState,
    commands: Vec<(String, Vec<u8>)>,
    ttl: Option<i64>,
) -> Result<(), StateStoreError> {
    let client = state.transport.async_client().await?;
    let mut pipeline = client.pipeline(false);
    for (key, bytes) in &commands {
        pipeline.commands.add_command(set_command(key, bytes, ttl));
    }
    pipeline
        .execute()
        .await
        .map(|_replies| ())
        .map_err(|source| StateStoreError::Redis {
            operation: "async_set_cache_pipeline",
            source,
        })
}

async fn add_members(
    state: &RedisState,
    key: &RouteKey,
    namespaced: String,
    values: Vec<String>,
    ttl: Option<i64>,
) -> Result<Vec<String>, StateStoreError> {
    let key_name = key.as_ref().to_owned();
    for _attempt in 0..UPDATE_ATTEMPTS {
        let current = read_one(state, key).await?;
        let mut members = match &current {
            StoreRead::Missing => BTreeSet::new(),
            StoreRead::Value(RouteValue::Members(members)) => members.clone(),
            StoreRead::Value(_) => {
                return Err(StateStoreError::Store(StoreError::NotMembers {
                    key: key_name.clone(),
                }));
            }
        };
        for value in &values {
            members.insert(value.clone());
        }
        let next = RouteValue::Members(members);
        let outcome = super::cas::compare_and_set(state, &namespaced, &current, &next, ttl).await?;
        if outcome.swapped {
            return Ok(values);
        }
    }
    Err(StateStoreError::Conflict { key: key_name })
}

async fn delete_one(state: &RedisState, key: String) -> Result<(), StateStoreError> {
    let client = state.transport.async_client().await?;
    let _deleted: i64 = client
        .query(&delete_command(&key))
        .await
        .map_err(|source| StateStoreError::Redis {
            operation: "async_delete_cache",
            source,
        })?;
    Ok(())
}

fn delete_command(key: &str) -> redis::Cmd {
    let mut command = cmd("DEL");
    command.arg(key);
    command
}

async fn expire_one(state: &RedisState, key: String, ttl: i64) -> Result<bool, StateStoreError> {
    let client = state.transport.async_client().await?;
    client
        .expire(&key, ttl)
        .await
        .map_err(|source| StateStoreError::Redis {
            operation: "async_refresh_ttl",
            source,
        })
}

async fn increment_until_written(
    state: &RedisState,
    namespaced: String,
    key_name: String,
    by: &RouteIncrement,
    ttl: Option<i64>,
) -> Result<RouteValue, StateStoreError> {
    for _attempt in 0..UPDATE_ATTEMPTS {
        if let Some(value) = super::cas::increment(state, &namespaced, by, ttl).await? {
            return Ok(value);
        }
    }
    Err(StateStoreError::Conflict { key: key_name })
}
