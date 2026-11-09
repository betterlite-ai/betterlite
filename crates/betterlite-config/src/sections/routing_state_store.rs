use std::str::FromStr;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_secret, resolve_string};
use crate::sections::Section;

const DEFAULT_SOCKET_TIMEOUT: f64 = 5.0;
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
    #[serde(default, rename = "redis_kwargs")]
    kwargs: RawKwargs,
}

#[derive(Debug, Default, Deserialize)]
struct RawKwargs {
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    socket_timeout: Option<RawNumber>,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub redis_url: Option<SecretString>,
    pub redis_host: Option<String>,
    pub redis_port: Option<i64>,
    pub redis_password: Option<SecretString>,
    pub namespace: Option<String>,
    pub socket_timeout: f64,
    pub pool_timeout_seconds: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            redis_url: None,
            redis_host: None,
            redis_port: None,
            redis_password: None,
            namespace: None,
            socket_timeout: DEFAULT_SOCKET_TIMEOUT,
            pool_timeout_seconds: DEFAULT_POOL_TIMEOUT,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let settings = self.router_settings;
        let host = optional_string(settings.host, "router_settings.redis_host", lookup)?;
        let explicit_url = resolve_secret(settings.url, "router_settings.redis_url", lookup)?;
        let explicit_port = resolve_number::<i64, _>(
            settings.port,
            "router_settings.redis_port",
            lookup,
            parse_i64,
        )?;
        let explicit_password =
            resolve_secret(settings.password, "router_settings.redis_password", lookup)?;
        let namespace = optional_string(
            settings.kwargs.namespace,
            "router_settings.redis_kwargs.namespace",
            lookup,
        )?;
        Ok(Settings {
            redis_url: configured_url(explicit_url, host.as_deref(), lookup),
            redis_host: host.or_else(|| lookup.lookup("REDIS_HOST")),
            redis_port: configured_port(explicit_port, lookup)?,
            redis_password: explicit_password
                .or_else(|| lookup.lookup("REDIS_PASSWORD").map(SecretString::from)),
            namespace,
            socket_timeout: configured_socket_timeout(settings.kwargs.socket_timeout, lookup)?,
            pool_timeout_seconds: environment_number(
                lookup,
                "REDIS_CONNECTION_POOL_TIMEOUT",
                DEFAULT_POOL_TIMEOUT,
            )?,
        })
    }
}

fn optional_string<L>(
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

fn configured_url<L>(
    explicit: Option<SecretString>,
    host: Option<&str>,
    lookup: &L,
) -> Option<SecretString>
where
    L: EnvironmentLookup,
{
    match explicit {
        Some(url) => Some(url),
        None if host.is_some() => None,
        None => lookup.lookup("REDIS_URL").map(SecretString::from),
    }
}

fn configured_port<L>(explicit: Option<i64>, lookup: &L) -> Result<Option<i64>, ConfigError>
where
    L: EnvironmentLookup,
{
    match explicit {
        Some(port) => Ok(Some(port)),
        None => match lookup.lookup("REDIS_PORT") {
            None => Ok(None),
            Some(text) => Ok(betterlite_env::number::<i64>("REDIS_PORT", &text)?),
        },
    }
}

fn configured_socket_timeout<L>(raw: Option<RawNumber>, lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    match raw {
        Some(value) => Ok(resolve_number::<f64, _>(
            Some(value),
            "redis_kwargs.socket_timeout",
            lookup,
            parse_float,
        )?
        .unwrap_or(DEFAULT_SOCKET_TIMEOUT)),
        None => environment_number(lookup, "REDIS_SOCKET_TIMEOUT", DEFAULT_SOCKET_TIMEOUT),
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

fn environment_number<L, T>(lookup: &L, name: &str, fallback: T) -> Result<T, ConfigError>
where
    L: EnvironmentLookup,
    T: FromStr,
{
    match lookup.lookup(name) {
        None => Ok(fallback),
        Some(text) => Ok(betterlite_env::number::<T>(name, &text)?.unwrap_or(fallback)),
    }
}

mod tests;
