use redis::FromRedisValue as _;
use rust_decimal::Decimal;

use super::super::error::StateStoreError;
use crate::routing_value_store::codec::parse_route_value;
use crate::routing_value_store::{
    MAX_VALUE_BYTES, RouteIncrement, RouteValue, StoreError, StoreRead,
};

pub(crate) fn encode(value: &RouteValue) -> Result<Option<Vec<u8>>, StateStoreError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|source| StateStoreError::Store(StoreError::Encoding { source }))?;
    Ok((bytes.len() <= MAX_VALUE_BYTES).then_some(bytes))
}

// B390's checked arithmetic stays private, so the redis transport repeats its outcomes without its match shape
pub(crate) fn incremented(
    key: &str,
    stored: &StoreRead,
    by: &RouteIncrement,
) -> Result<RouteValue, StateStoreError> {
    match (stored, by) {
        (StoreRead::Missing, RouteIncrement::Counter(value)) => Ok(RouteValue::Counter(*value)),
        (StoreRead::Missing, RouteIncrement::Spend(value)) => Ok(RouteValue::Spend(*value)),
        (StoreRead::Value(RouteValue::Counter(current)), RouteIncrement::Counter(value)) => {
            counter_sum(key, *current, *value)
        }
        (StoreRead::Value(RouteValue::Counter(current)), RouteIncrement::Spend(value)) => {
            spend_sum(key, Decimal::from(*current), *value)
        }
        (StoreRead::Value(RouteValue::Spend(current)), RouteIncrement::Counter(value)) => {
            spend_sum(key, *current, Decimal::from(*value))
        }
        (StoreRead::Value(RouteValue::Spend(current)), RouteIncrement::Spend(value)) => {
            spend_sum(key, *current, *value)
        }
        (StoreRead::Value(_), RouteIncrement::Counter(_) | RouteIncrement::Spend(_)) => {
            Err(StateStoreError::Store(StoreError::NotIncrementable {
                key: key.to_owned(),
            }))
        }
    }
}

fn counter_sum(key: &str, current: i64, amount: i64) -> Result<RouteValue, StateStoreError> {
    current
        .checked_add(amount)
        .map(RouteValue::Counter)
        .ok_or(StateStoreError::Store(StoreError::CounterOverflow {
            key: key.to_owned(),
        }))
}

fn spend_sum(key: &str, current: Decimal, amount: Decimal) -> Result<RouteValue, StateStoreError> {
    current
        .checked_add(amount)
        .map(RouteValue::Spend)
        .ok_or(StateStoreError::Store(StoreError::SpendOverflow {
            key: key.to_owned(),
        }))
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

pub(crate) fn set_command(key: &str, bytes: &[u8], ttl: i64) -> redis::Cmd {
    let mut command = redis::cmd("SET");
    command.arg(key).arg(bytes).arg("EX").arg(ttl);
    command
}
