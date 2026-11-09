use betterlite_redis::redis_live_client::{LiveClientError, LiveEndpoint, LiveTarget};
use redis::aio::MultiplexedConnection;
use redis::{AsyncConnectionConfig, Value, cmd};
use tokio::sync::Mutex;

use super::super::error::StateStoreError;
use super::RedisState;
use super::codec::{decode, encode, set_command};
use crate::routing_value_store::{CompareOutcome, RouteValue, StoreRead};

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
    let current = watched_read(connection, key).await?;
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
    if write_watched(connection, key, &bytes, ttl).await? {
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
) -> Result<StoreRead, StateStoreError> {
    let mut pipeline = redis::Pipeline::new();
    pipeline.cmd("WATCH").arg(key);
    pipeline.cmd("GET").arg(key);
    let (_watched, raw): (String, Option<Vec<u8>>) = pipeline
        .query_async(connection)
        .await
        .map_err(|source| StateStoreError::Redis {
            operation: "watch",
            source: LiveClientError::Redis(source),
        })?;
    decode(raw)
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
    ttl: Option<i64>,
) -> Result<bool, StateStoreError> {
    let mut pipeline = redis::Pipeline::new();
    pipeline.atomic();
    pipeline.add_command(set_command(key, bytes, ttl));
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
