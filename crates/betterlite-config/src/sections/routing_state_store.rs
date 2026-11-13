use std::collections::BTreeMap;
use std::str::FromStr;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{resolve_number, resolve_secret, resolve_string};
use crate::sections::Section;

use self::connection::Connection;
use self::kwargs::{Presence, RawKwargs, kwargs, parse_i64};

pub mod connection;
mod environment;
mod kwargs;

const DEFAULT_SOCKET_TIMEOUT: f64 = 5.0;
const SHARED_CLIENT_SOCKET_TIMEOUT: f64 = 0.1;
const DEFAULT_POOL_TIMEOUT: u32 = 5;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouterSettings,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawRouterSettings {
    #[serde(rename = "redis_url")]
    url: Option<String>,
    #[serde(rename = "redis_host")]
    host: Option<String>,
    #[serde(rename = "redis_port")]
    port: Option<RawNumber>,
    #[serde(rename = "redis_password")]
    password: Option<SecretString>,
    #[serde(rename = "redis_db")]
    db: Option<RawNumber>,
    cache_kwargs: RawKwargs,
    redis_kwargs: RawKwargs,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub redis_url: Option<SecretString>,
    pub redis_host: Option<String>,
    pub redis_port: Option<i64>,
    pub redis_password: Option<SecretString>,
    pub namespace: Option<String>,
    pub connection: Connection,
    pub socket_timeout: f64,
    pub pool_timeout_seconds: u32,
    pub redis_environment: BTreeMap<String, SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let router = self.router_settings;
        let cache = kwargs(router.cache_kwargs, "cache_kwargs", lookup)?;
        let redis = kwargs(router.redis_kwargs, "redis_kwargs", lookup)?;
        let shared_timeout = number::<f64, _>(lookup, "REDIS_SOCKET_TIMEOUT")?
            .unwrap_or(SHARED_CLIENT_SOCKET_TIMEOUT);
        let socket_timeout = match cache.socket_timeout.or(redis.socket_timeout) {
            Presence::Absent => DEFAULT_SOCKET_TIMEOUT,
            Presence::Null(()) => shared_timeout,
            Presence::Value(seconds) => seconds,
        };
        let port =
            resolve_number::<i64, _>(router.port, "router_settings.redis_port", lookup, parse_i64)?;
        let legacy_db =
            resolve_number::<i64, _>(router.db, "router_settings.redis_db", lookup, parse_i64)?;
        let db = cache.connection.db.or(legacy_db).or(redis.connection.db);
        let mut connection = cache.connection.or(redis.connection);
        connection.db = db;
        Ok(Settings {
            redis_url: resolve_secret(router.url, "router_settings.redis_url", lookup)?,
            redis_host: optional_text(router.host, "router_settings.redis_host", lookup)?,
            redis_port: port,
            redis_password: resolve_secret(
                router.password,
                "router_settings.redis_password",
                lookup,
            )?,
            namespace: cache.namespace.or(redis.namespace),
            connection,
            socket_timeout,
            pool_timeout_seconds: number::<u32, _>(lookup, "REDIS_CONNECTION_POOL_TIMEOUT")?
                .unwrap_or(DEFAULT_POOL_TIMEOUT),
            redis_environment: environment::collect(lookup),
        })
    }
}

fn optional_text<L>(
    raw: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    raw.map(|text| resolve_string(text, path, lookup))
        .transpose()
}

fn number<T, L>(lookup: &L, name: &str) -> Result<Option<T>, ConfigError>
where
    L: EnvironmentLookup,
    T: FromStr,
{
    match lookup.lookup(name) {
        None => Ok(None),
        Some(text) => Ok(betterlite_env::number::<T>(name, &text)?),
    }
}

mod tests;
