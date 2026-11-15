use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::guardrail_params::base::UnreachableFallback;

pub const DEFAULT_API_BASE: &str = "https://api.alice.io";
pub const API_KEY_ENV: &str = "ALICE_API_KEY";
pub const API_BASE_ENV: &str = "ALICE_API_BASE";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    guardrails: Vec<Entry>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
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
    unreachable_fallback: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub guardrails: Vec<Configured>,
}

#[derive(Debug, Clone)]
pub struct Configured {
    pub guardrail_name: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub unreachable_fallback: UnreachableFallback,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            if entry.litellm_params.guardrail == "alice" {
                guardrails.push(entry.resolve(index, lookup)?);
            }
        }
        Ok(Settings { guardrails })
    }
}

impl Entry {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("guardrails.{index}.litellm_params");
        let configured_key = self
            .litellm_params
            .api_key
            .filter(|key| !key.expose_secret().is_empty());
        let api_key = match configured_key {
            Some(key) => Some(SecretString::from(resolve_string(
                key.expose_secret().to_owned(),
                &format!("{prefix}.api_key"),
                lookup,
            )?)),
            None => lookup
                .lookup(API_KEY_ENV)
                .filter(|key| !key.is_empty())
                .map(SecretString::from),
        };
        let api_base = match self
            .litellm_params
            .api_base
            .filter(|value| !value.is_empty())
        {
            Some(value) => Some(resolve_string(
                value,
                &format!("{prefix}.api_base"),
                lookup,
            )?),
            None => lookup
                .lookup(API_BASE_ENV)
                .filter(|value| !value.is_empty()),
        };
        let fallback_path = format!("{prefix}.unreachable_fallback");
        let unreachable_fallback = match self.litellm_params.unreachable_fallback {
            Some(value) => parse_unreachable_fallback(&value, &fallback_path)?,
            None => UnreachableFallback::default(),
        };
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_key,
            api_base,
            unreachable_fallback,
        })
    }
}

fn parse_unreachable_fallback(value: &str, path: &str) -> Result<UnreachableFallback, ConfigError> {
    match value.to_lowercase().as_str() {
        "fail_closed" => Ok(UnreachableFallback::FailClosed),
        "fail_open" => Ok(UnreachableFallback::FailOpen),
        _ => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }),
    }
}

mod tests;
