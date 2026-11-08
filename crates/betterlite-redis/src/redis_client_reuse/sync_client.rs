use std::sync::{Arc, Mutex, MutexGuard};

use redis::cluster::ClusterConnection;
use redis::{Cmd, Connection, FromRedisValue};

use super::error::ClientReuseError;
use crate::redis_client_factories::RedisClient;
use crate::redis_live_client::{LiveClientError, LiveClientFactory, LiveTarget};

// litellm.caching.redis_cluster_cache: re-creating a client costs 3000 ms, so one connection is kept.
pub struct LiveSyncClient {
    pub(crate) factory: Arc<LiveClientFactory>,
    pub(crate) client: RedisClient,
    pub(crate) connection: Mutex<Option<SyncConnection>>,
}

pub(crate) enum SyncConnection {
    Standalone(Connection),
    Cluster(Box<ClusterConnection>),
}

impl LiveSyncClient {
    // Blocks on network I/O; callers on async threads must go through spawn_blocking.
    pub fn query<T>(&self, command: &Cmd) -> Result<T, ClientReuseError>
    where
        T: FromRedisValue,
    {
        query_locked(
            &mut locked(&self.connection),
            &self.factory,
            &self.client,
            command,
        )
    }
}

fn locked(connection: &Mutex<Option<SyncConnection>>) -> MutexGuard<'_, Option<SyncConnection>> {
    match connection.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn query_locked<T>(
    slot: &mut Option<SyncConnection>,
    factory: &LiveClientFactory,
    client: &RedisClient,
    command: &Cmd,
) -> Result<T, ClientReuseError>
where
    T: FromRedisValue,
{
    let mut connection = match slot.take() {
        Some(connection) => connection,
        None => open_connection(factory, client)?,
    };
    let reply = match &mut connection {
        SyncConnection::Standalone(inner) => command.query(inner),
        SyncConnection::Cluster(inner) => command.query(inner),
    };
    match reply {
        Ok(value) => {
            *slot = Some(connection);
            Ok(value)
        }
        Err(error) => {
            if !error.is_connection_dropped() {
                *slot = Some(connection);
            }
            Err(ClientReuseError::from(LiveClientError::Redis(error)))
        }
    }
}

fn open_connection(
    factory: &LiveClientFactory,
    record: &RedisClient,
) -> Result<SyncConnection, ClientReuseError> {
    let endpoint = factory.blocking_endpoint(record)?;
    match endpoint.target {
        LiveTarget::Standalone(endpoint_client) => {
            let connection = endpoint_client
                .get_connection_with_timeout(endpoint.connect_timeout)
                .map_err(LiveClientError::Redis)?;
            connection
                .set_read_timeout(Some(endpoint.response_timeout))
                .map_err(LiveClientError::Redis)?;
            connection
                .set_write_timeout(Some(endpoint.response_timeout))
                .map_err(LiveClientError::Redis)?;
            Ok(SyncConnection::Standalone(connection))
        }
        LiveTarget::Cluster(cluster) => {
            let connection = cluster.get_connection().map_err(LiveClientError::Redis)?;
            connection
                .set_read_timeout(Some(endpoint.response_timeout))
                .map_err(LiveClientError::Redis)?;
            connection
                .set_write_timeout(Some(endpoint.response_timeout))
                .map_err(LiveClientError::Redis)?;
            Ok(SyncConnection::Cluster(Box::new(connection)))
        }
    }
}
