use std::collections::BTreeMap;
use std::str::FromStr;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_secret, resolve_string};
use crate::sections::Section;

mod environment;

const DEFAULT_SOCKET_TIMEOUT: f64 = 5.0;
const SHARED_CLIENT_SOCKET_TIMEOUT: f64 = 0.1;
const DEFAULT_POOL_TIMEOUT: u32 = 5;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouterSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouterSettings {
    #[serde(default, rename = "redis_url")]
    url: Option<String>,
    #[serde(default, rename = "redis_host")]
    host: Option<String>,
    #[serde(default, rename = "redis_port")]
    port: Option<RawNumber>,
    #[serde(default, rename = "redis_password")]
    password: Option<SecretString>,
    #[serde(default)]
    cache_kwargs: RawKwargs,
    #[serde(default)]
    redis_kwargs: RawKwargs,
}

#[derive(Debug, Default, Deserialize)]
struct RawKwargs {
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    socket_timeout: Presence<RawNumber>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum Presence<T> {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Null(()),
    Value(T),
}

impl<T> Presence<T> {
    fn or(self, other: Self) -> Self {
        match self {
            Self::Absent => other,
            set @ (Self::Null(()) | Self::Value(_)) => set,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub redis_url: Option<SecretString>,
    pub redis_host: Option<String>,
    pub redis_port: Option<i64>,
    pub redis_password: Option<SecretString>,
    pub namespace: Option<String>,
    pub socket_timeout: f64,
    pub pool_timeout_seconds: u32,
    pub redis_environment: BTreeMap<String, SecretString>,
}

struct Kwargs {
    namespace: Option<String>,
    socket_timeout: Presence<f64>,
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
            socket_timeout,
            pool_timeout_seconds: number::<u32, _>(lookup, "REDIS_CONNECTION_POOL_TIMEOUT")?
                .unwrap_or(DEFAULT_POOL_TIMEOUT),
            redis_environment: environment::collect(lookup),
        })
    }
}

fn kwargs<L>(raw: RawKwargs, section: &str, lookup: &L) -> Result<Kwargs, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = |key: &str| format!("router_settings.{section}.{key}");
    let socket_timeout = match raw.socket_timeout {
        Presence::Absent => Presence::Absent,
        Presence::Null(()) => Presence::Null(()),
        Presence::Value(number) => {
            resolve_number::<f64, _>(Some(number), &path("socket_timeout"), lookup, parse_float)?
                .map_or(Presence::Null(()), Presence::Value)
        }
    };
    Ok(Kwargs {
        namespace: optional_text(raw.namespace, &path("namespace"), lookup)?,
        socket_timeout,
    })
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

fn parse_i64(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.trim()
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}

mod tests;
