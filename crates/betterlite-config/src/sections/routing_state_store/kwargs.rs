use serde::Deserialize;

use super::connection::Connection;
use super::optional_text;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawKwargs {
    namespace: Option<String>,
    socket_timeout: Presence<RawNumber>,
    db: Option<RawNumber>,
    username: Option<String>,
    ssl: Option<bool>,
    socket_connect_timeout: Option<RawNumber>,
    socket_keepalive: Option<bool>,
    max_connections: Option<RawNumber>,
    health_check_interval: Option<RawNumber>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
pub(super) enum Presence<T> {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Null(()),
    Value(T),
}

impl<T> Presence<T> {
    pub(super) fn or(self, other: Self) -> Self {
        match self {
            Self::Absent => other,
            set @ (Self::Null(()) | Self::Value(_)) => set,
        }
    }
}

pub(super) struct Kwargs {
    pub(super) namespace: Option<String>,
    pub(super) socket_timeout: Presence<f64>,
    pub(super) connection: Connection,
}

pub(super) fn kwargs<L>(raw: RawKwargs, section: &str, lookup: &L) -> Result<Kwargs, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = |key: &str| format!("router_settings.{section}.{key}");
    let socket_timeout = resolve_timeout(raw.socket_timeout, &path("socket_timeout"), lookup)?;
    let connection = Connection {
        db: resolve_number(raw.db, &path("db"), lookup, parse_i64)?,
        username: optional_text(raw.username, &path("username"), lookup)?,
        ssl: raw.ssl,
        socket_connect_timeout: resolve_number(
            raw.socket_connect_timeout,
            &path("socket_connect_timeout"),
            lookup,
            parse_float,
        )?,
        socket_keepalive: raw.socket_keepalive,
        max_connections: resolve_number(
            raw.max_connections,
            &path("max_connections"),
            lookup,
            parse_i64,
        )?,
        health_check_interval: resolve_number(
            raw.health_check_interval,
            &path("health_check_interval"),
            lookup,
            parse_i64,
        )?,
    };
    Ok(Kwargs {
        namespace: optional_text(raw.namespace, &path("namespace"), lookup)?,
        socket_timeout,
        connection,
    })
}

fn resolve_timeout<L>(
    raw: Presence<RawNumber>,
    path: &str,
    lookup: &L,
) -> Result<Presence<f64>, ConfigError>
where
    L: EnvironmentLookup,
{
    Ok(match raw {
        Presence::Absent => Presence::Absent,
        Presence::Null(()) => Presence::Null(()),
        Presence::Value(number) => {
            resolve_number::<f64, _>(Some(number), path, lookup, parse_float)?
                .map_or(Presence::Null(()), Presence::Value)
        }
    })
}

pub(super) fn parse_i64(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.trim()
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
