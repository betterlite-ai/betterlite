use betterlite_redis::redis_live_client::{LiveClientError, LiveEndpoint, LiveTarget};
use redis::aio::MultiplexedConnection;
use redis::{AsyncConnectionConfig, Value, cmd};
use tokio::sync::Mutex;

use super::super::error::StateStoreError;
use super::RedisState;
use super::codec::{decode, encode, incremented, set_command};
use crate::routing_value_store::{CompareOutcome, RouteIncrement, RouteValue, StoreRead};

pub(crate) struct CasConnection {
    pub(crate) endpoint: LiveEndpoint,
    pub(crate) slot: Mutex<Option<MultiplexedConnection>>,
}

pub(crate) async fn compare_and_set(
    state: &RedisState,
    key: &str,
    expected: &StoreRead,
    value: &RouteValue,
    ttl: Option<i64>,
) -> Result<CompareOutcome, StateStoreError> {
    let mut slot = state.cas.slot.lock().await;
    let outcome = {
        let connection = connection(&mut slot, &state.cas.endpoint).await?;
        compare_on(connection, key, expected, value, ttl).await
    };
    if outcome.is_err() {
        *slot = None;
    }
    outcome
}

async fn compare_on(
    connection: &mut MultiplexedConnection,
    key: &str,
    expected: &StoreRead,
    value: &RouteValue,
    ttl: Option<i64>,
) -> Result<CompareOutcome, StateStoreError> {
    let (current, _expiry) = watched_read(connection, key, false).await?;
    if current != *expected {
        unwatch(connection).await?;
        return Ok(CompareOutcome {
            swapped: false,
            current,
        });
    }
    let Some(bytes) = encode(key, value)? else {
        unwatch(connection).await?;
        return Ok(CompareOutcome {
            swapped: false,
            current,
        });
    };
    let expiry = ttl.map_or(Expiry::None, Expiry::Set);
    if write_watched(connection, key, &bytes, expiry).await? {
        return Ok(CompareOutcome {
            swapped: true,
            current: StoreRead::Value(value.clone()),
        });
    }
    let latest = read_plain(connection, key).await?;
    Ok(CompareOutcome {
        swapped: false,
        current: latest,
    })
}

pub(crate) async fn increment(
    state: &RedisState,
    key: &str,
    by: &RouteIncrement,
    ttl: Option<i64>,
) -> Result<Option<RouteValue>, StateStoreError> {
    let mut slot = state.cas.slot.lock().await;
    let outcome = {
        let connection = connection(&mut slot, &state.cas.endpoint).await?;
        increment_on(connection, key, by, ttl).await
    };
    if outcome.is_err() {
        *slot = None;
    }
    outcome
}

async fn increment_on(
    connection: &mut MultiplexedConnection,
    key: &str,
    by: &RouteIncrement,
    ttl: Option<i64>,
) -> Result<Option<RouteValue>, StateStoreError> {
    let (current, ttl_reply) = watched_read(connection, key, true).await?;
    let next = incremented(key, &current, by)?;
    let Some(bytes) = encode(key, &next)? else {
        unwatch(connection).await?;
        return Ok(Some(next));
    };
    let expiry = expiry_plan(ttl, ttl_reply);
    if write_watched(connection, key, &bytes, expiry).await? {
        return Ok(Some(next));
    }
    Ok(None)
}

#[derive(Clone, Copy)]
enum Expiry {
    None,
    Set(i64),
    Keep(i64),
}

// a live ttl survives an increment, exactly as INCRBYFLOAT leaves it untouched
const fn expiry_plan(ttl: Option<i64>, ttl_reply: i64) -> Expiry {
    match ttl {
        Some(seconds) if ttl_reply < 0 => Expiry::Set(seconds),
        Some(_) => Expiry::Keep(ttl_reply),
        None if ttl_reply >= 0 => Expiry::Keep(ttl_reply),
        None => Expiry::None,
    }
}

async fn connection<'slot>(
    slot: &'slot mut Option<MultiplexedConnection>,
    endpoint: &LiveEndpoint,
) -> Result<&'slot mut MultiplexedConnection, StateStoreError> {
    if slot.is_none() {
        *slot = Some(open_connection(endpoint).await?);
    }
    slot.as_mut().ok_or(StateStoreError::NoConnection)
}

async fn open_connection(
    endpoint: &LiveEndpoint,
) -> Result<MultiplexedConnection, StateStoreError> {
    let LiveTarget::Standalone(client) = &endpoint.target else {
        return Err(StateStoreError::UnsupportedTarget);
    };
    let config = AsyncConnectionConfig::new()
        .set_connection_timeout(Some(endpoint.connect_timeout))
        .set_response_timeout(Some(endpoint.response_timeout));
    client
        .get_multiplexed_async_connection_with_config(&config)
        .await
        .map_err(|source| StateStoreError::Redis {
            operation: "connect",
            source: LiveClientError::Redis(source),
        })
}

async fn watched_read(
    connection: &mut MultiplexedConnection,
    key: &str,
    with_ttl: bool,
) -> Result<(StoreRead, i64), StateStoreError> {
    let mut pipeline = redis::Pipeline::new();
    pipeline.cmd("WATCH").arg(key);
    pipeline.cmd("GET").arg(key);
    if with_ttl {
        pipeline.cmd("TTL").arg(key);
        let (_watched, raw, ttl): (String, Option<Vec<u8>>, i64) = pipeline
            .query_async(connection)
            .await
            .map_err(|source| StateStoreError::Redis {
                operation: "watch",
                source: LiveClientError::Redis(source),
            })?;
        return Ok((decode(raw)?, ttl));
    }
    let (_watched, raw): (String, Option<Vec<u8>>) = pipeline
        .query_async(connection)
        .await
        .map_err(|source| StateStoreError::Redis {
            operation: "watch",
            source: LiveClientError::Redis(source),
        })?;
    Ok((decode(raw)?, -2))
}

async fn read_plain(
    connection: &mut MultiplexedConnection,
    key: &str,
) -> Result<StoreRead, StateStoreError> {
    let raw: Option<Vec<u8>> =
        cmd("GET")
            .arg(key)
            .query_async(connection)
            .await
            .map_err(|source| StateStoreError::Redis {
                operation: "compare_and_set",
                source: LiveClientError::Redis(source),
            })?;
    decode(raw)
}

async fn write_watched(
    connection: &mut MultiplexedConnection,
    key: &str,
    bytes: &[u8],
    expiry: Expiry,
) -> Result<bool, StateStoreError> {
    let mut pipeline = redis::Pipeline::new();
    pipeline.atomic();
    match expiry {
        Expiry::None => pipeline.add_command(set_command(key, bytes, None)),
        Expiry::Set(seconds) => pipeline.add_command(set_command(key, bytes, Some(seconds))),
        Expiry::Keep(seconds) => {
            pipeline.add_command(set_command(key, bytes, None));
            let mut expire_command = cmd("EXPIRE");
            expire_command.arg(key).arg(seconds);
            pipeline.add_command(expire_command)
        }
    };
    let executed: Option<Value> =
        pipeline
            .query_async(connection)
            .await
            .map_err(|source| StateStoreError::Redis {
                operation: "transaction",
                source: LiveClientError::Redis(source),
            })?;
    Ok(executed.is_some())
}

async fn unwatch(connection: &mut MultiplexedConnection) -> Result<(), StateStoreError> {
    cmd("UNWATCH")
        .query_async::<()>(connection)
        .await
        .map_err(|source| StateStoreError::Redis {
            operation: "unwatch",
            source: LiveClientError::Redis(source),
        })
}
