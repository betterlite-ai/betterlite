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
        let raw = self.router_settings;
        let target = resolve_target(raw.host, raw.url, raw.port, raw.password, lookup)?;
        Ok(Settings {
            redis_url: target.url,
            redis_host: target.host,
            redis_port: target.port,
            redis_password: target.password,
            namespace: optional_text(
                raw.kwargs.namespace,
                "router_settings.redis_kwargs.namespace",
                lookup,
            )?,
            socket_timeout: socket_timeout(raw.kwargs.socket_timeout, lookup)?,
            pool_timeout_seconds: number::<u32, _>(lookup, "REDIS_CONNECTION_POOL_TIMEOUT")?
                .unwrap_or(DEFAULT_POOL_TIMEOUT),
        })
    }
}

struct Target {
    url: Option<SecretString>,
    host: Option<String>,
    port: Option<i64>,
    password: Option<SecretString>,
}

fn resolve_target<L>(
    explicit_host: Option<String>,
    explicit_url: Option<String>,
    explicit_port: Option<RawNumber>,
    explicit_password: Option<SecretString>,
    lookup: &L,
) -> Result<Target, ConfigError>
where
    L: EnvironmentLookup,
{
    let host = optional_text(explicit_host, "router_settings.redis_host", lookup)?
        .or_else(|| lookup.lookup("REDIS_HOST"));
    let url = resolve_secret(explicit_url, "router_settings.redis_url", lookup)?.or_else(|| {
        if host.is_some() {
            None
        } else {
            lookup.lookup("REDIS_URL").map(SecretString::from)
        }
    });
    let port = match resolve_number::<i64, _>(
        explicit_port,
        "router_settings.redis_port",
        lookup,
        parse_i64,
    )? {
        Some(port) => Some(port),
        None => number::<i64, _>(lookup, "REDIS_PORT")?,
    };
    let password = resolve_secret(explicit_password, "router_settings.redis_password", lookup)?
        .or_else(|| lookup.lookup("REDIS_PASSWORD").map(SecretString::from));
    Ok(Target {
        url,
        host,
        port,
        password,
    })
}

fn socket_timeout<L>(raw: Option<RawNumber>, lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    match raw {
        Some(value) => Ok(resolve_number::<f64, _>(
            Some(value),
            "router_settings.redis_kwargs.socket_timeout",
            lookup,
            parse_float,
        )?
        .unwrap_or(DEFAULT_SOCKET_TIMEOUT)),
        None => {
            Ok(number::<f64, _>(lookup, "REDIS_SOCKET_TIMEOUT")?.unwrap_or(DEFAULT_SOCKET_TIMEOUT))
        }
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

fn parse_i64(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.trim()
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}

mod tests;
