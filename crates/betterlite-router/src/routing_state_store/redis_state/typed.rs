use betterlite_redis::cache_primitives::protocol::AsyncRedisCommands as _;
use redis::cmd;

use super::super::error::StateStoreError;
use super::RedisState;
use super::codec::{decode, encode, incremented};
use crate::routing_value_store::{RouteIncrement, RouteValue, StoreRead};

const UPDATE_ATTEMPTS: usize = 8;

async fn read_one(state: &RedisState, namespaced: &str) -> Result<StoreRead, StateStoreError> {
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
    ttl: Option<i64>,
    key_name: String,
) -> Result<RouteValue, StateStoreError> {
    for _attempt in 0..UPDATE_ATTEMPTS {
        let current = read_one(state, namespaced).await?;
        let remaining = read_ttl(state, namespaced).await?;
        let next = incremented(namespaced, &current, by)?;
        if encode(namespaced, &next)?.is_none() {
            return Ok(next);
        }
        let expiry = if remaining >= 0 { Some(remaining) } else { ttl };
        let outcome =
            super::cas::compare_and_set(state, namespaced, &current, &next, expiry).await?;
        if outcome.swapped {
            return Ok(next);
        }
    }
    Err(StateStoreError::Conflict { key: key_name })
}
