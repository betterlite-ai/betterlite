use redis::aio::ConnectionManagerConfig;
use redis::cluster::ClusterClient;
use redis::cluster_read_routing::RandomReplicaStrategy;
use redis::io::tcp::TcpSettings;
use redis::{Client, ConnectionAddr, ConnectionInfo, IntoConnectionInfo as _, RedisConnectionInfo};
use secrecy::{ExposeSecret as _, SecretString};
use socket2::TcpKeepalive;

use super::connection_info::{self, Timeouts};
use super::credentials::{RedisCredentials, SourceProvider};
use super::tls::{self, TlsSettings, TlsState};
use super::{LiveClientError, LiveTarget};
use crate::redis_client_factories::cluster::ClusterClient as ClusterRecord;
use crate::redis_client_factories::fields::ConnectionFields;
use crate::redis_client_factories::url::UrlClient;
use crate::redis_connection_settings::model::scalars::PassthroughValue;

// redis-py retries no connection attempt; the manager keeps its retry budget at one attempt.
const NO_RETRIES: usize = 0;

pub(crate) struct StandalonePlan {
    info: ConnectionInfo,
    tls: TlsSettings,
    pub(crate) timeouts: Timeouts,
    limit: usize,
    pub(crate) source: Option<SourceProvider>,
}

impl StandalonePlan {
    pub(crate) fn manager_config(&self) -> ConnectionManagerConfig {
        let mut config = ConnectionManagerConfig::new()
            .set_response_timeout(Some(self.timeouts.response))
            .set_connection_timeout(Some(self.timeouts.connect))
            .set_concurrency_limit(self.limit)
            .set_number_of_retries(NO_RETRIES);
        if let Some(source) = &self.source {
            config = config.set_credentials_provider(source.clone());
        }
        config
    }
}

pub(crate) trait ConnectionPlan {
    type Target;

    fn source(&self) -> Option<&SourceProvider>;
    fn timeouts(&self) -> Timeouts;
    fn client(
        &self,
        credentials: Option<&RedisCredentials>,
    ) -> Result<Self::Target, LiveClientError>;
    fn target(client: Self::Target) -> LiveTarget;
}

impl ConnectionPlan for StandalonePlan {
    type Target = Client;

    fn source(&self) -> Option<&SourceProvider> {
        self.source.as_ref()
    }

    fn timeouts(&self) -> Timeouts {
        self.timeouts
    }

    fn target(client: Self::Target) -> LiveTarget {
        LiveTarget::Standalone(Box::new(client))
    }

    fn client(&self, credentials: Option<&RedisCredentials>) -> Result<Client, LiveClientError> {
        let Some(fetched) = credentials else {
            return tls::client(self.info.clone(), &self.tls);
        };
        let mut redis = self.info.redis_settings().clone();
        if let Some(username) = &fetched.username {
            redis = redis.set_username(username);
        }
        redis = redis.set_password(fetched.password.expose_secret());
        tls::client(self.info.clone().set_redis_settings(redis), &self.tls)
    }
}

pub(crate) struct ClusterPlan {
    nodes: Vec<ConnectionInfo>,
    tls: TlsSettings,
    pub(crate) timeouts: Timeouts,
    limit: usize,
    pub(crate) source: Option<SourceProvider>,
    username: Option<String>,
    password: Option<SecretString>,
    socket_keepalive: bool,
    read_from_replicas: bool,
}

impl ConnectionPlan for ClusterPlan {
    type Target = ClusterClient;

    fn source(&self) -> Option<&SourceProvider> {
        self.source.as_ref()
    }

    fn timeouts(&self) -> Timeouts {
        self.timeouts
    }

    fn target(client: Self::Target) -> LiveTarget {
        LiveTarget::Cluster(Box::new(client))
    }

    fn client(
        &self,
        credentials: Option<&RedisCredentials>,
    ) -> Result<ClusterClient, LiveClientError> {
        let mut builder = ClusterClient::builder(self.nodes.clone())
            .connection_timeout(self.timeouts.connect)
            .response_timeout(self.timeouts.response)
            .connection_concurrency_limit(self.limit);
        if self.socket_keepalive {
            builder =
                builder.tcp_settings(TcpSettings::default().set_keepalive(TcpKeepalive::new()));
        }
        if self.read_from_replicas {
            builder = builder.read_routing_strategy(RandomReplicaStrategy);
        }
        if let Some(fetched) = credentials {
            builder = builder.password(fetched.password.expose_secret());
            if let Some(username) = &fetched.username {
                builder = builder.username(username);
            }
        } else {
            if let Some(username) = &self.username {
                builder = builder.username(username);
            }
            if let Some(password) = &self.password {
                builder = builder.password(password.expose_secret());
            }
        }
        let configured = tls::configure_cluster(builder, &self.tls);
        let server = if let (Some(source), None) = (&self.source, credentials) {
            configured.set_credentials_provider(source.clone())
        } else {
            configured
        };
        Ok(server.build()?)
    }
}

pub(crate) fn plan_standalone(
    fields: &ConnectionFields,
    source: Option<SourceProvider>,
    limit: usize,
) -> Result<StandalonePlan, LiveClientError> {
    tls::warn_unmapped(&fields.passthrough, &[]);
    let mut tls_settings = tls::from_passthrough(&fields.passthrough)?;
    if fields.ssl == Some(true) {
        tls_settings.state = TlsState::Enabled;
    }
    let timeouts = connection_info::timeouts(fields.socket_timeout, fields.socket_connect_timeout)?;
    let host = fields
        .host
        .as_deref()
        .unwrap_or(connection_info::DEFAULT_HOST);
    let port = connection_info::port(fields.port)?;
    let (username, password) = match &source {
        Some(_) => (None, None),
        None => (fields.username.as_deref(), fields.password.as_ref()),
    };
    let base = connection_info::info(host, port, fields.db.unwrap_or(0), username, password)?;
    let info = connection_info::with_keepalive(base, fields.socket_keepalive);
    Ok(StandalonePlan {
        info,
        tls: tls_settings,
        timeouts,
        limit,
        source,
    })
}

pub(crate) fn plan_url(
    record: &UrlClient,
    source: Option<SourceProvider>,
    limit: usize,
) -> Result<StandalonePlan, LiveClientError> {
    tls::warn_unmapped(&record.passthrough, &[]);
    let url: &str = record.url.expose_secret();
    let mut connection = url.into_connection_info()?;
    if let Some(username) = &record.username {
        let redis = connection.redis_settings().clone().set_username(username);
        connection = connection.set_redis_settings(redis);
    }
    if source.is_some() {
        let redis = RedisConnectionInfo::default()
            .set_db(connection.redis_settings().db())
            .set_protocol(connection.redis_settings().protocol());
        connection = connection.set_redis_settings(redis);
    }
    let tls_settings = tls::from_url(&record.url, &record.passthrough)?;
    let timeouts = connection_info::timeouts(record.socket_timeout, record.socket_connect_timeout)?;
    let info = connection_info::with_keepalive(connection, record.socket_keepalive);
    Ok(StandalonePlan {
        info,
        tls: tls_settings,
        timeouts,
        limit,
        source,
    })
}

pub(crate) fn plan_cluster(
    record: &ClusterRecord,
    source: Option<SourceProvider>,
    limit: usize,
) -> Result<ClusterPlan, LiveClientError> {
    tls::warn_unmapped(&record.passthrough, &["read_from_replicas"]);
    let mut tls_settings = tls::from_passthrough(&record.passthrough)?;
    if record.ssl == Some(true) {
        tls_settings.state = TlsState::Enabled;
    }
    let timeouts = connection_info::timeouts(record.socket_timeout, record.socket_connect_timeout)?;
    let mut nodes = Vec::with_capacity(record.startup_nodes.len());
    for node in &record.startup_nodes {
        let port = u16::try_from(node.port)
            .map_err(|_source| LiveClientError::InvalidSetting { field: "port" })?;
        nodes.push(ConnectionAddr::Tcp(node.host.clone(), port).into_connection_info()?);
    }
    let (username, password) = match &source {
        Some(_) => (None, None),
        None => (record.username.clone(), record.password.clone()),
    };
    Ok(ClusterPlan {
        nodes,
        tls: tls_settings,
        timeouts,
        limit,
        source,
        username,
        password,
        socket_keepalive: record.socket_keepalive == Some(true),
        read_from_replicas: matches!(
            record.passthrough.get("read_from_replicas"),
            Some(PassthroughValue::Bool(true))
        ),
    })
}
