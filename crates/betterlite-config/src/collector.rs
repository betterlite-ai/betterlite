use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::time::Duration;

use rust_decimal::{Decimal, prelude::ToPrimitive as _};
use serde::{Deserialize, Serialize};

mod tests;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnavailablePolicy {
    #[default]
    Fallback,
    Drop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RawSettings")]
pub struct CollectorSettings {
    pub enabled: bool,
    pub address: String,
    pub buffer_size: usize,
    pub on_unavailable: UnavailablePolicy,
    pub drain_timeout_seconds: Decimal,
    pub connect_timeout_seconds: Decimal,
    pub job_role: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSettings {
    enabled: Option<bool>,
    address: Option<String>,
    buffer_size: Option<usize>,
    on_unavailable: Option<UnavailablePolicy>,
    drain_timeout_seconds: Option<Decimal>,
    connect_timeout_seconds: Option<Decimal>,
    job_role: Option<String>,
}
impl From<RawSettings> for CollectorSettings {
    fn from(raw: RawSettings) -> Self {
        let defaults = Self::default();
        Self {
            enabled: raw.enabled.unwrap_or(defaults.enabled),
            address: raw.address.unwrap_or(defaults.address),
            buffer_size: raw.buffer_size.unwrap_or(defaults.buffer_size),
            on_unavailable: raw.on_unavailable.unwrap_or(defaults.on_unavailable),
            drain_timeout_seconds: raw
                .drain_timeout_seconds
                .unwrap_or(defaults.drain_timeout_seconds),
            connect_timeout_seconds: raw
                .connect_timeout_seconds
                .unwrap_or(defaults.connect_timeout_seconds),
            job_role: raw.job_role,
        }
    }
}

impl Default for CollectorSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            address: "unix:///var/run/litellm/collector.sock".to_owned(),
            buffer_size: 1000,
            on_unavailable: UnavailablePolicy::Fallback,
            drain_timeout_seconds: Decimal::TEN,
            connect_timeout_seconds: Decimal::ONE,
            job_role: None,
        }
    }
}

impl CollectorSettings {
    pub fn validate(&self) -> Result<(Duration, Duration), SettingsError> {
        if self.buffer_size == 0 || self.buffer_size > (usize::MAX >> 3_u32) {
            return Err(SettingsError::Value("buffer_size".to_owned()));
        }
        Ok((
            duration(self.drain_timeout_seconds, "drain_timeout_seconds")?,
            duration(self.connect_timeout_seconds, "connect_timeout_seconds")?,
        ))
    }

    fn assign(&mut self, name: &str, value: &str) -> Result<(), SettingsError> {
        match name {
            "enabled" => self.enabled = betterlite_env::flag(name, value)?.unwrap_or(false),
            "address" => value.clone_into(&mut self.address),
            "buffer_size" => {
                self.buffer_size = betterlite_env::number(name, value)?.unwrap_or(self.buffer_size);
            }
            "on_unavailable" => {
                self.on_unavailable = match value {
                    "fallback" => UnavailablePolicy::Fallback,
                    "drop" => UnavailablePolicy::Drop,
                    _ => return Err(SettingsError::Value(name.to_owned())),
                }
            }
            "drain_timeout_seconds" => {
                self.drain_timeout_seconds =
                    betterlite_env::number(name, value)?.unwrap_or(self.drain_timeout_seconds);
            }
            "connect_timeout_seconds" => {
                self.connect_timeout_seconds =
                    betterlite_env::number(name, value)?.unwrap_or(self.connect_timeout_seconds);
            }
            _ => return Err(SettingsError::Value(name.to_owned())),
        }
        Ok(())
    }
}

fn duration(seconds: Decimal, field: &str) -> Result<Duration, SettingsError> {
    let nanos = seconds
        .checked_mul(Decimal::from(1_000_000_000_u64))
        .and_then(|value| value.to_u64());
    match nanos {
        Some(value) if value > 0 => Ok(Duration::from_nanos(value)),
        Some(_) | None => Err(SettingsError::Value(field.to_owned())),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("invalid collector {0}")]
    Value(String),
    #[error(transparent)]
    Environment(#[from] betterlite_env::EnvError),
}

pub fn load_collector_settings(
    environment: &BTreeMap<String, String>,
) -> Result<CollectorSettings, SettingsError> {
    let mut settings = CollectorSettings::default();
    let mut seen = BTreeSet::new();
    for (name, value) in environment {
        if value.trim().is_empty() {
            continue;
        }
        let lowercase = name.to_ascii_lowercase();
        let field = if lowercase == "litellm_job_role" {
            "job_role"
        } else if let Some(field) = lowercase.strip_prefix("litellm_collector_") {
            field
        } else {
            continue;
        };
        if !seen.insert(field.to_owned()) {
            return Err(SettingsError::Value(format!("duplicate {field}")));
        }
        if lowercase == "litellm_job_role" {
            settings.job_role = Some(value.clone());
        } else {
            settings.assign(field, value)?;
        }
    }
    settings.validate()?;
    Ok(settings)
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TracebackPolicy {
    pub suppress: bool,
}

pub fn load_spend_traceback_policy(
    value: Option<&str>,
) -> Result<TracebackPolicy, betterlite_env::EnvError> {
    let suppress = betterlite_env::flag(
        "LITELLM_SUPPRESS_SPEND_LOG_TRACEBACKS",
        value.unwrap_or_default(),
    )?
    .unwrap_or(false);
    Ok(TracebackPolicy { suppress })
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CollectorConfigInput {
    Settings { config: CollectorSettings },
    Address { value: String },
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
    let input = serde_json::from_slice(bytes)?;
    if let CollectorConfigInput::Settings { config } = &input {
        config.validate()?;
    }
    Ok(input)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectorAddress {
    Unix { path: String },
    Tcp { host: String, port: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    #[error("invalid collector URL syntax")]
    Syntax(#[source] url::ParseError),
    #[error("tcp collector address must use loopback")]
    RemoteHost,
    #[error("expected unix:///path or tcp://127.0.0.1:port")]
    Invalid,
}

pub fn parse_collector_address(address: &str) -> Result<CollectorAddress, AddressError> {
    let Some((scheme, rest)) = address.split_once(':') else {
        return Err(AddressError::Invalid);
    };
    if scheme.eq_ignore_ascii_case("unix") {
        let suffix = rest.split(['?', '#']).next().unwrap_or("");
        if suffix.contains('[') {
            url::Url::parse(address).map_err(AddressError::Syntax)?;
        }
        let path = suffix.strip_prefix("//").map_or(suffix, |authority| {
            authority
                .find('/')
                .and_then(|offset| authority.get(offset..))
                .unwrap_or("")
        });
        if !path.is_empty() {
            return Ok(CollectorAddress::Unix {
                path: path.to_owned(),
            });
        }
        return Err(AddressError::Invalid);
    }
    let parsed = url::Url::parse(address).map_err(AddressError::Syntax)?;
    if parsed.scheme() != "tcp" {
        return Err(AddressError::Invalid);
    }
    let host = parsed
        .host_str()
        .ok_or(AddressError::Invalid)?
        .trim_matches(['[', ']'])
        .to_ascii_lowercase();
    let port = parsed.port().ok_or(AddressError::Invalid)?;
    if host != "localhost" && !host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()) {
        return Err(AddressError::RemoteHost);
    }
    Ok(CollectorAddress::Tcp { host, port })
}
