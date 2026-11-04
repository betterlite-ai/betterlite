use std::net::IpAddr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnavailablePolicy {
    Fallback,
    Drop,
}

impl Default for UnavailablePolicy {
    fn default() -> Self {
        Self::Fallback
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CollectorSettings {
    pub enabled: bool,
    pub address: String,
    pub buffer_size: usize,
    pub on_unavailable: UnavailablePolicy,
    pub drain_timeout_seconds: f64,
    pub connect_timeout_seconds: f64,
    pub job_role: Option<String>,
}

impl Default for CollectorSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            address: "unix:///var/run/litellm/collector.sock".to_owned(),
            buffer_size: 1000,
            on_unavailable: UnavailablePolicy::Fallback,
            drain_timeout_seconds: 10.0,
            connect_timeout_seconds: 1.0,
            job_role: None,
        }
    }
}

impl CollectorSettings {
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.buffer_size == 0 {
            return Err(SettingsError::BufferSize);
        }
        if !self.drain_timeout_seconds.is_finite() || self.drain_timeout_seconds <= 0.0 {
            return Err(SettingsError::DrainTimeout);
        }
        if !self.connect_timeout_seconds.is_finite() || self.connect_timeout_seconds <= 0.0 {
            return Err(SettingsError::ConnectTimeout);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    #[error("collector buffer_size must be at least one")]
    BufferSize,
    #[error("collector drain_timeout_seconds must be positive")]
    DrainTimeout,
    #[error("collector connect_timeout_seconds must be positive")]
    ConnectTimeout,
    #[error("invalid collector environment variable {0}")]
    Environment(String),
}

pub fn load_collector_settings<L>(lookup: L) -> Result<CollectorSettings, SettingsError>
where
    L: Fn(&str) -> Option<String>,
{
    let mut result = CollectorSettings::default();
    for (name, value) in [
        "LITELLM_COLLECTOR_ENABLED",
        "LITELLM_COLLECTOR_ADDRESS",
        "LITELLM_COLLECTOR_BUFFER_SIZE",
        "LITELLM_COLLECTOR_ON_UNAVAILABLE",
        "LITELLM_COLLECTOR_DRAIN_TIMEOUT_SECONDS",
        "LITELLM_COLLECTOR_CONNECT_TIMEOUT_SECONDS",
        "LITELLM_JOB_ROLE",
    ]
    .into_iter()
    .filter_map(|name| {
        let lowercase = name.to_ascii_lowercase();
        lookup(name)
            .or_else(|| lookup(&lowercase))
            .map(|value| (name, value))
    }) {
        let invalid = || SettingsError::Environment(name.to_owned());
        match name {
            "LITELLM_COLLECTOR_ENABLED" => {
                result.enabled = betterlite_env::flag(name, &value)
                    .map_err(|_| invalid())?
                    .unwrap_or(result.enabled)
            }
            "LITELLM_COLLECTOR_ADDRESS" => result.address = value,
            "LITELLM_COLLECTOR_BUFFER_SIZE" => {
                result.buffer_size = betterlite_env::number::<usize>(name, &value)
                    .map_err(|_| invalid())?
                    .ok_or_else(invalid)?
            }
            "LITELLM_COLLECTOR_ON_UNAVAILABLE" => {
                result.on_unavailable = match value.to_ascii_lowercase().as_str() {
                    "fallback" => UnavailablePolicy::Fallback,
                    "drop" => UnavailablePolicy::Drop,
                    _ => return Err(invalid()),
                }
            }
            "LITELLM_COLLECTOR_DRAIN_TIMEOUT_SECONDS" => {
                result.drain_timeout_seconds = betterlite_env::number::<f64>(name, &value)
                    .map_err(|_| invalid())?
                    .ok_or_else(invalid)?
            }
            "LITELLM_COLLECTOR_CONNECT_TIMEOUT_SECONDS" => {
                result.connect_timeout_seconds = betterlite_env::number::<f64>(name, &value)
                    .map_err(|_| invalid())?
                    .ok_or_else(invalid)?
            }
            "LITELLM_JOB_ROLE" => result.job_role = (!value.is_empty()).then_some(value),
            _ => {}
        }
    }
    result.validate()?;
    Ok(result)
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CollectorConfigInput {
    Settings { config: CollectorSettings },
    Suppression { value: Option<String> },
}

#[derive(Debug, thiserror::Error)]
pub enum CollectorDecodeError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Settings(#[from] SettingsError),
}

pub fn decode_collector_settings(
    bytes: &[u8],
) -> Result<CollectorConfigInput, CollectorDecodeError> {
    match serde_json::from_slice(bytes)? {
        CollectorConfigInput::Settings { config } => {
            config.validate()?;
            Ok(CollectorConfigInput::Settings { config })
        }
        input @ CollectorConfigInput::Suppression { .. } => Ok(input),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectorAddress {
    Unix { path: String },
    Tcp { host: String, port: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    #[error("invalid collector URL syntax: {0}")]
    Syntax(String),
    #[error("tcp collector address must be a loopback host, got '{0}'")]
    RemoteHost(String),
    #[error("expected unix:///path or tcp://127.0.0.1:port")]
    Invalid,
}

pub fn parse_collector_address(address: &str) -> Result<CollectorAddress, AddressError> {
    let parsed =
        url::Url::parse(address).map_err(|error| AddressError::Syntax(error.to_string()))?;
    match parsed.scheme() {
        "unix" if !parsed.path().is_empty() => Ok(CollectorAddress::Unix {
            path: parsed.path().to_owned(),
        }),
        "tcp" => {
            let host = parsed.host_str().ok_or(AddressError::Invalid)?;
            let port = parsed.port().ok_or(AddressError::Invalid)?;
            let loopback =
                host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
            if !loopback {
                return Err(AddressError::RemoteHost(address.to_owned()));
            }
            Ok(CollectorAddress::Tcp {
                host: host.to_owned(),
                port,
            })
        }
        _ => Err(AddressError::Invalid),
    }
}

#[path = "collector/tests.rs"]
mod collector_tests;
