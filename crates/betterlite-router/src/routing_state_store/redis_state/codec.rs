use std::sync::Arc;

use redis::FromRedisValue as _;

use super::super::error::StateStoreError;
use crate::routing_value_store::codec::parse_route_value;
use crate::routing_value_store::{
    MAX_VALUE_BYTES, RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreError,
    StoreRead, StoreSettings,
};

pub(crate) fn encode(value: &RouteValue) -> Result<Option<Vec<u8>>, StateStoreError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|source| StateStoreError::Store(StoreError::Encoding { source }))?;
    Ok((bytes.len() <= MAX_VALUE_BYTES).then_some(bytes))
}

// B390 keeps its checked arithmetic private, so a scratch store runs the same increment on the value read from redis
pub(crate) fn incremented(
    key: &RouteKey,
    stored: &StoreRead,
    by: &RouteIncrement,
) -> Result<RouteValue, StateStoreError> {
    let settings = StoreSettings {
        max_size_in_memory: None,
        default_ttl_seconds: None,
        max_size_per_item: None,
        loaded_max_size_per_item_kib: 1024,
    };
    let scratch = RoutingValueStore::new(settings, Arc::new(|| 0.0_f64))?;
    if let StoreRead::Value(current) = stored {
        scratch.set(key, current, None)?;
    }
    Ok(scratch.increment(key, by, None)?)
}

pub(crate) fn decode(raw: Option<Vec<u8>>) -> Result<StoreRead, StateStoreError> {
    raw.map_or(Ok(StoreRead::Missing), |bytes| {
        parse_route_value(&bytes)
            .map(StoreRead::Value)
            .map_err(StateStoreError::Store)
    })
}

pub(crate) fn reply_bytes(
    operation: &'static str,
    value: redis::Value,
) -> Result<Option<Vec<u8>>, StateStoreError> {
    Option::<Vec<u8>>::from_redis_value(value)
        .map_err(|source| StateStoreError::Reply { operation, source })
}

#[derive(Clone, Copy)]
pub(crate) enum Expiry {
    Seconds(i64),
    Keep,
}

pub(crate) fn set_command(key: &str, bytes: &[u8], expiry: Expiry) -> redis::Cmd {
    let mut command = redis::cmd("SET");
    command.arg(key).arg(bytes);
    match expiry {
        Expiry::Seconds(ttl) => command.arg("EX").arg(ttl),
        Expiry::Keep => command.arg("KEEPTTL"),
    };
    command
}
