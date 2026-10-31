use std::collections::BTreeMap;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use thiserror::Error;

mod timeout;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

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
    unreachable_fallback: UnreachableFallback,
    timeout: timeout::Timeout,
    extra_headers: Option<ExtraHeaders>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ExtraHeaders {
    Mapping(BTreeMap<String, SecretString>),
    List(Vec<String>),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub enum UnreachableFallback {
    #[default]
    FailClosed,
    FailOpen,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid unreachable_fallback: {value}; expected fail_open or fail_closed")]
    Fallback { value: String },
}

impl TryFrom<String> for UnreachableFallback {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.to_lowercase().as_str() {
            "fail_closed" => Ok(Self::FailClosed),
            "fail_open" => Ok(Self::FailOpen),
            _ => Err(Error::Fallback { value }),
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
    pub extra_headers: BTreeMap<String, SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            match entry.litellm_params.guardrail.as_str() {
                "deepkeep" => guardrails.push(entry.resolve(index, lookup)?),
                // the lasso_classification section reads these entries
                "lasso" => {}
                _ => {
                    return Err(ConfigError::Unsupported {
                        paths: vec![format!("guardrails.{index}.litellm_params.guardrail")],
                    });
                }
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
        let parameters = self.litellm_params;
        let api_key = SecretString::from(required_value(
            parameters
                .api_key
                .map(|secret| secret.expose_secret().to_owned()),
            "DEEPKEEP_API_KEY",
            &prefix,
            "api_key",
            lookup,
        )?);
        let api_base = required_value(
            parameters.api_base,
            "DEEPKEEP_API_BASE",
            &prefix,
            "api_base",
            lookup,
        )?;
        let firewall_id = required_value(
            parameters.deepkeep_firewall_id,
            "DEEPKEEP_FIREWALL_ID",
            &prefix,
            "deepkeep_firewall_id",
            lookup,
        )?;
        let extra_headers = resolve_extra_headers(parameters.extra_headers);
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_base: normalize_endpoint(&api_base),
            api_key,
            deepkeep_firewall_id: firewall_id,
            unreachable_fallback: parameters.unreachable_fallback,
            timeout: parameters.timeout.0,
            extra_headers,
        })
    }
}

fn resolve_extra_headers(extra_headers: Option<ExtraHeaders>) -> BTreeMap<String, SecretString> {
    match extra_headers {
        Some(ExtraHeaders::Mapping(values)) => values,
        Some(ExtraHeaders::List(values)) => {
            drop(values);
            tracing::warn!("DeepKeep extra_headers list ignored; a mapping is required");
            BTreeMap::new()
        }
        None => BTreeMap::new(),
    }
}

fn required_value<L>(
    configured: Option<String>,
    variable: &str,
    prefix: &str,
    path_name: &str,
    lookup: &L,
) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = format!("{prefix}.{path_name}");
    let missing = || ConfigError::MissingEnvironment {
        variable: variable.to_owned(),
        path: path.clone(),
    };
    let candidate = configured
        .filter(|value| !value.is_empty())
        .or_else(|| lookup.lookup(variable))
        .ok_or_else(missing)?;
    let resolved = resolve_string(candidate, &path, lookup)?;
    if resolved.is_empty() {
        return Err(missing());
    }
    Ok(resolved)
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
