use std::time::Instant;

use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use betterlite_redis::cache_primitives::protocol::AsyncRedisCommands as _;
use redis::cmd;

use super::super::error::StateStoreError;
use super::RedisState;
use super::codec::{Expiry, decode, encode, incremented, reply_bytes};
use crate::routing_value_store::{RouteIncrement, RouteKey, RouteValue, StoreRead};

pub(crate) async fn read_one(
    state: &RedisState,
    namespaced: &str,
) -> Result<StoreRead, StateStoreError> {
    let client = state.transport.async_client().await?;
    let mut command = cmd("GET");
    command.arg(namespaced);
    let reply: Option<Vec<u8>> =
        client
            .query(&command)
            .await
            .map_err(|source| StateStoreError::Redis {
                operation: "async_increment",
                source,
            })?;
    decode(reply)
}

pub(crate) async fn read_ttl(state: &RedisState, namespaced: &str) -> Result<i64, StateStoreError> {
    let client = state.transport.async_client().await?;
    client
        .ttl(namespaced)
        .await
        .map_err(|source| StateStoreError::Redis {
            operation: "async_increment",
            source,
        })
}

pub(crate) async fn increment_until_written(
    state: &RedisState,
    namespaced: &str,
    by: &RouteIncrement,
    ttl: i64,
    key: &RouteKey,
) -> Result<RouteValue, StateStoreError> {
    let started = Instant::now();
    loop {
        let current = read_one(state, namespaced).await?;
        let remaining = read_ttl(state, namespaced).await?;
        let next = incremented(key, &current, by)?;
        if encode(&next)?.is_none() {
            return Ok(next);
        }
        let expiry = if remaining >= 0 {
            Expiry::Keep
        } else {
            Expiry::Seconds(ttl)
        };
        let outcome =
            super::cas::compare_and_set(state, namespaced, &current, &next, expiry).await?;
        if outcome.swapped {
            return Ok(next);
        }
        if started.elapsed() >= state.update_window {
            return Err(StateStoreError::Conflict {
                key: key.as_ref().to_owned(),
            });
        }
    }
}

pub(crate) async fn read_batch(
    state: &RedisState,
    namespaced: &[String],
) -> Result<Vec<StoreRead>, StateStoreError> {
    let replies = state.transport.async_mget(namespaced).await?;
    let mut values = Vec::with_capacity(replies.len());
    for reply in replies {
        values.push(decode(reply_bytes("async_batch_get_cache", reply)?)?);
    }
    Ok(values)
}

pub(crate) async fn increment_float(
    state: &RedisState,
    namespaced: &str,
    value: f64,
    ttl: i64,
    refresh: bool,
) -> Result<f64, StateStoreError> {
    state
        .counters
        .incr_by_float_with_ttl(namespaced, value, Some(ttl), refresh)
        .await
        .map_err(|source| StateStoreError::Counters {
            operation: "async_increment",
            source,
        })
}

pub(crate) fn pipeline_operations(
    state: &RedisState,
    entries: &[(RouteKey, f64, Option<f64>)],
) -> Result<Vec<IncrementOperation>, StateStoreError> {
    let mut operations = Vec::with_capacity(entries.len());
    for (key, value, ttl_seconds) in entries {
        let ttl = ttl_seconds
            .map(|seconds| {
                super::truncate_seconds(seconds).ok_or(StateStoreError::InvalidTtl { seconds })
            })
            .transpose()?;
        operations.push(IncrementOperation {
            key: super::namespaced(state, key),
            increment_value: *value,
            ttl,
        });
    }
    Ok(operations)
}

pub(crate) async fn increment_pipeline(
    state: &RedisState,
    operations: &[IncrementOperation],
) -> Result<Vec<f64>, StateStoreError> {
    state
        .counters
        .pipeline_increment(operations)
        .await
        .map_err(|source| StateStoreError::Counters {
            operation: "async_increment_pipeline",
            source,
        })
}
