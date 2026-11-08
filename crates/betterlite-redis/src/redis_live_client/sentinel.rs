use std::sync::{Arc, Mutex};

use redis::aio::MultiplexedConnection;
use redis::{AsyncConnectionConfig, Client, Cmd, ConnectionInfo, FromRedisValue, RedisError, cmd};
use tokio::runtime::Handle;

use super::connection_info::{self, Timeouts};
use super::credentials::{self, RedisCredentials, SourceProvider};
use super::tls::{self, TlsSettings, TlsState};
use super::{LiveClientError, LiveEndpoint, LiveTarget};
use crate::redis_client_factories::sentinel::SentinelClient;

#[derive(Clone)]
pub struct SentinelConnection {
    record: Arc<SentinelClient>,
    credentials: Option<SourceProvider>,
    timeouts: Timeouts,
    slot: Arc<Mutex<Option<MultiplexedConnection>>>,
}

impl SentinelConnection {
    pub(crate) async fn open(
        record: SentinelClient,
        credentials: Option<SourceProvider>,
    ) -> Result<Self, LiveClientError> {
        let timeouts = connection_info::timeouts(
            record.connection_kwargs.socket_timeout,
            record.connection_kwargs.socket_connect_timeout,
        )?;
        let connection = Self {
            record: Arc::new(record),
            credentials,
            timeouts,
            slot: Arc::new(Mutex::new(None)),
        };
        let master = connection.open_master().await?;
        connection.store(master);
        Ok(connection)
    }

    pub(crate) async fn query<T>(&self, command: &Cmd) -> Result<T, LiveClientError>
    where
        T: FromRedisValue,
    {
        let mut connection = self.connection().await?;
        match command.query_async(&mut connection).await {
            Ok(value) => Ok(value),
            Err(error) => {
                self.reset_if_stale(&error);
                Err(LiveClientError::Redis(error))
            }
        }
    }

    pub(crate) async fn connection(&self) -> Result<MultiplexedConnection, LiveClientError> {
        if let Some(connection) = self.cached() {
            return Ok(connection);
        }
        let opened = self.open_master().await?;
        self.store(opened.clone());
        Ok(opened)
    }

    pub(crate) fn reset_if_stale(&self, error: &RedisError) {
        if error.is_connection_dropped() || error.code() == Some("READONLY") {
            self.clear();
        }
    }

    async fn open_master(&self) -> Result<MultiplexedConnection, LiveClientError> {
        let record = Arc::clone(&self.record);
        let use_provider = self.credentials.is_some();
        let client = Handle::current()
            .spawn_blocking(move || driver_client(&record, use_provider))
            .await??;
        let mut config = AsyncConnectionConfig::new()
            .set_connection_timeout(Some(self.timeouts.connect))
            .set_response_timeout(Some(self.timeouts.response));
        if let Some(source) = &self.credentials {
            config = config.set_credentials_provider(source.clone());
        }
        Ok(client
            .get_multiplexed_async_connection_with_config(&config)
            .await?)
    }

    fn cached(&self) -> Option<MultiplexedConnection> {
        let slot = match self.slot.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        slot.clone()
    }

    fn store(&self, connection: MultiplexedConnection) {
        let mut slot = match self.slot.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *slot = Some(connection);
    }

    fn clear(&self) {
        let mut slot = match self.slot.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *slot = None;
    }
}

pub(crate) fn blocking_endpoint(
    record: &SentinelClient,
    injected: Option<&SourceProvider>,
) -> Result<LiveEndpoint, LiveClientError> {
    let needed = record.connection_kwargs.credential_provider
        || record
            .connection_kwargs
            .credential_provider_settings
            .is_some();
    let source = credentials::required(injected, needed)?;
    tls::warn_unmapped(&record.connection_kwargs.passthrough, &[]);
    let mut tls_settings = tls::from_passthrough(&record.connection_kwargs.passthrough)?;
    if record.connection_kwargs.ssl == Some(true) {
        tls_settings.state = TlsState::Enabled;
    }
    let timeouts = connection_info::timeouts(
        record.connection_kwargs.socket_timeout,
        record.connection_kwargs.socket_connect_timeout,
    )?;
    let master = resolve_master(record, &tls_settings)?;
    let fetched = credentials::fetch(source.as_ref())?;
    let info = master_info(record, &master, fetched.as_ref())?;
    let client = tls::client(info, &tls_settings)?;
    Ok(LiveEndpoint {
        target: LiveTarget::Standalone(Box::new(client)),
        connect_timeout: timeouts.connect,
        response_timeout: timeouts.response,
    })
}

fn driver_client(record: &SentinelClient, use_provider: bool) -> Result<Client, LiveClientError> {
    tls::warn_unmapped(&record.connection_kwargs.passthrough, &[]);
    let mut tls_settings = tls::from_passthrough(&record.connection_kwargs.passthrough)?;
    if record.connection_kwargs.ssl == Some(true) {
        tls_settings.state = TlsState::Enabled;
    }
    let master = resolve_master(record, &tls_settings)?;
    let fields = &record.connection_kwargs;
    let (username, password) = if use_provider {
        (None, None)
    } else {
        (fields.username.as_deref(), fields.password.as_ref())
    };
    let base = connection_info::info(
        &master.host,
        master.port,
        fields.db.unwrap_or(0),
        username,
        password,
    )?;
    let info = connection_info::with_keepalive(base, fields.socket_keepalive);
    tls::client(info, &tls_settings)
}

fn master_info(
    record: &SentinelClient,
    master: &Master,
    credentials: Option<&RedisCredentials>,
) -> Result<ConnectionInfo, LiveClientError> {
    let fields = &record.connection_kwargs;
    let (username, password) = credentials.map_or(
        (fields.username.as_deref(), fields.password.as_ref()),
        |fetched| (fetched.username.as_deref(), Some(&fetched.password)),
    );
    let info = connection_info::info(
        &master.host,
        master.port,
        fields.db.unwrap_or(0),
        username,
        password,
    )?;
    Ok(connection_info::with_keepalive(
        info,
        fields.socket_keepalive,
    ))
}

struct Master {
    host: String,
    port: u16,
}

fn resolve_master(
    record: &SentinelClient,
    tls_settings: &TlsSettings,
) -> Result<Master, LiveClientError> {
    let timeouts = connection_info::timeouts(
        record.connection_kwargs.socket_timeout,
        record.connection_kwargs.socket_connect_timeout,
    )?;
    let mut monitors = Vec::with_capacity(record.sentinel_nodes.len());
    for (host, port) in &record.sentinel_nodes {
        let resolved = u16::try_from(*port)
            .map_err(|_source| LiveClientError::InvalidSetting { field: "port" })?;
        monitors.push((host, resolved));
    }
    for (host, port) in monitors {
        match master_from_monitor(record, tls_settings, host, port, timeouts) {
            Ok(Some(master)) => return Ok(master),
            Ok(None) | Err(_) => {
                tracing::warn!(host = %host, port = port, "redis sentinel monitor did not report the master address");
            }
        }
    }
    Err(LiveClientError::NoSentinelMaster {
        service_name: record.service_name.clone(),
    })
}

fn master_from_monitor(
    record: &SentinelClient,
    tls_settings: &TlsSettings,
    host: &str,
    port: u16,
    timeouts: Timeouts,
) -> Result<Option<Master>, LiveClientError> {
    let fields = &record.connection_kwargs;
    let info = connection_info::info(
        host,
        port,
        0,
        fields.username.as_deref(),
        record.monitor_password.as_ref(),
    )?;
    let client = tls::client(info, tls_settings)?;
    let mut connection = client.get_connection_with_timeout(timeouts.connect)?;
    connection.set_read_timeout(Some(timeouts.response))?;
    connection.set_write_timeout(Some(timeouts.response))?;
    let answer: Option<(String, String)> = cmd("SENTINEL")
        .arg("GET-MASTER-ADDR-BY-NAME")
        .arg(&record.service_name)
        .query(&mut connection)?;
    answer.map_or(Ok(None), |(master_host, master_port)| {
        master_port.parse::<u16>().map_or_else(
            |_source| Ok(None),
            |resolved| {
                Ok(Some(Master {
                    host: master_host,
                    port: resolved,
                }))
            },
        )
    })
}
