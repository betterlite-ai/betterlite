use std::time::Duration;

use secrecy::SecretString;
use serde::Deserialize;
use serde::de::IgnoredAny;
use thiserror::Error;

mod resolve;
mod timeout;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};

pub const UI_FRIENDLY_NAME: &str = "DeepKeep AI Firewall";
pub const ENDPOINT: &str = "/v3/openai/beta/litellm_basic_guardrail_api";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    guardrails: Vec<Entry>,
}

#[derive(Debug, Default, Deserialize)]
struct Entry {
    guardrail_name: String,
    litellm_params: Parameters,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Parameters {
    guardrail: String,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    deepkeep_firewall_id: Option<String>,
    unreachable_fallback: Option<String>,
    timeout: Option<timeout::Seconds>,
    extra_headers: Option<RawExtraHeaders>,
    optional_params: Option<OptionalParams>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct OptionalParams {
    unreachable_fallback: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawExtraHeaders {
    List(Vec<String>),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UnreachableFallback {
    #[default]
    FailClosed,
    FailOpen,
}

#[derive(Debug, Error)]
pub enum FallbackError {
    #[error("invalid unreachable_fallback: {value}; expected fail_open or fail_closed")]
    Fallback { value: String },
}

impl TryFrom<String> for UnreachableFallback {
    type Error = FallbackError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.to_lowercase().as_str() {
            "fail_closed" => Ok(Self::FailClosed),
            "fail_open" => Ok(Self::FailOpen),
            _ => Err(FallbackError::Fallback { value }),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub guardrails: Vec<Configured>,
}

#[derive(Debug, Clone)]
pub struct Configured {
    pub guardrail_name: String,
    pub api_key: SecretString,
    pub api_base: String,
    pub deepkeep_firewall_id: String,
    pub unreachable_fallback: UnreachableFallback,
    pub timeout: Duration,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            if entry.litellm_params.guardrail == "deepkeep" {
                guardrails.push(entry.resolve(index, lookup)?);
            }
        }
        Ok(Settings { guardrails })
    }
}

fn yaml_error<M>(path: &str, message: M) -> ConfigError
where
    M: std::fmt::Display,
{
    ConfigError::Yaml {
        source: <serde_saphyr::Error as serde::de::Error>::custom(format!("{path}: {message}")),
    }
}

#[must_use]
pub fn normalize_endpoint(api_base: &str) -> String {
    let base = api_base.trim_end_matches('/');
    if base.ends_with(ENDPOINT) {
        base.to_owned()
    } else {
        format!("{base}{ENDPOINT}")
    }
}

#[path = "deepkeep_firewall/tests.rs"]
mod tests;
