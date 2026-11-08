use std::time::Duration;

use redis::io::tcp::TcpSettings;
use redis::{ConnectionAddr, ConnectionInfo, IntoConnectionInfo as _, RedisConnectionInfo};
use secrecy::{ExposeSecret as _, SecretString};
use socket2::TcpKeepalive;

use super::LiveClientError;

pub(crate) const DEFAULT_HOST: &str = "localhost";
const DEFAULT_PORT: u16 = 6379;
const DEFAULT_TIMEOUT_SECONDS: u64 = 5;
const DEFAULT_CONCURRENCY_LIMIT: usize = 1024;

#[derive(Clone, Copy)]
pub(crate) struct Timeouts {
    pub(crate) connect: Duration,
    pub(crate) response: Duration,
}

pub(crate) fn timeouts(
    socket_timeout: Option<f64>,
    socket_connect_timeout: Option<f64>,
) -> Result<Timeouts, LiveClientError> {
    let response = seconds("socket_timeout", socket_timeout)?;
    let connect = seconds("socket_connect_timeout", socket_connect_timeout)?.or(response);
    Ok(Timeouts {
        connect: connect.unwrap_or_else(|| Duration::from_secs(DEFAULT_TIMEOUT_SECONDS)),
        response: response.unwrap_or_else(|| Duration::from_secs(DEFAULT_TIMEOUT_SECONDS)),
    })
}

fn seconds(field: &'static str, value: Option<f64>) -> Result<Option<Duration>, LiveClientError> {
    value.map_or_else(
        || Ok(None),
        |timeout| {
            Duration::try_from_secs_f64(timeout)
                .map(Some)
                .map_err(|_source| LiveClientError::InvalidSetting { field })
        },
    )
}

pub(crate) fn port(record_port: Option<i64>) -> Result<u16, LiveClientError> {
    if let Some(value) = record_port {
        return u16::try_from(value)
            .map_err(|_source| LiveClientError::InvalidSetting { field: "port" });
    }
    Ok(DEFAULT_PORT)
}

pub(crate) fn concurrency_limit(
    record: Option<i64>,
    pool: Option<u64>,
) -> Result<usize, LiveClientError> {
    if let Some(limit) = pool {
        return usize::try_from(limit).map_err(|_source| LiveClientError::InvalidSetting {
            field: "max_connections",
        });
    }
    record.map_or_else(
        || Ok(DEFAULT_CONCURRENCY_LIMIT),
        |value| {
            usize::try_from(value).map_err(|_source| LiveClientError::InvalidSetting {
                field: "max_connections",
            })
        },
    )
}

pub(crate) fn info(
    host: &str,
    port: u16,
    db: i64,
    username: Option<&str>,
    password: Option<&SecretString>,
) -> Result<ConnectionInfo, LiveClientError> {
    let mut redis = RedisConnectionInfo::default().set_db(db);
    if let Some(name) = username {
        redis = redis.set_username(name);
    }
    if let Some(secret) = password {
        redis = redis.set_password(secret.expose_secret());
    }
    Ok(ConnectionAddr::Tcp(host.to_owned(), port)
        .into_connection_info()?
        .set_redis_settings(redis))
}

pub(crate) fn with_keepalive(info: ConnectionInfo, keepalive: Option<bool>) -> ConnectionInfo {
    if keepalive == Some(true) {
        return info.set_tcp_settings(TcpSettings::default().set_keepalive(TcpKeepalive::new()));
    }
    info
}
