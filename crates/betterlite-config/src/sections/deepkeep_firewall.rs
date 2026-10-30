use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

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
    optional_params: OptionalParameters,
    extra_headers: Option<ExtraHeaders>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ExtraHeaders {
    Mapping(BTreeMap<String, String>),
    List(Vec<String>),
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct OptionalParameters {
    unreachable_fallback: Option<UnreachableFallback>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnreachableFallback {
    #[default]
    FailClosed,
    FailOpen,
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
    pub extra_headers: BTreeMap<String, String>,
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
            } else {
                return Err(ConfigError::Unsupported {
                    paths: vec![format!("guardrails.{index}.litellm_params.guardrail")],
                });
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
        let api_key = required_secret(parameters.api_key, "DEEPKEEP_API_KEY", &prefix, lookup)?;
        let api_base = required_string(parameters.api_base, "DEEPKEEP_API_BASE", &prefix, lookup)?;
        let firewall_id = required_string(
            parameters.deepkeep_firewall_id,
            "DEEPKEEP_FIREWALL_ID",
            &prefix,
            lookup,
        )?;
        let extra_headers = match parameters.extra_headers {
            Some(ExtraHeaders::Mapping(values)) => values,
            Some(ExtraHeaders::List(values)) => {
                drop(values);
                tracing::warn!("DeepKeep extra_headers list ignored; a mapping is required");
                BTreeMap::new()
            }
            _ => BTreeMap::new(),
        };
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_base: normalize_endpoint(&api_base),
            api_key,
            deepkeep_firewall_id: firewall_id,
            unreachable_fallback: parameters
                .optional_params
                .unreachable_fallback
                .unwrap_or_default(),
            extra_headers,
        })
    }
}

fn required_secret<L>(
    configured: Option<SecretString>,
    variable: &str,
    prefix: &str,
    lookup: &L,
) -> Result<SecretString, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolved = configured
        .filter(|secret| !secret.expose_secret().is_empty())
        .map(|secret| secret.expose_secret().to_owned())
        .or_else(|| lookup.lookup(variable))
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: variable.to_owned(),
            path: format!("{prefix}.api_key"),
        })?;
    Ok(SecretString::from(resolve_string(
        resolved,
        &format!("{prefix}.api_key"),
        lookup,
    )?))
}

fn required_string<L>(
    configured: Option<String>,
    variable: &str,
    prefix: &str,
    lookup: &L,
) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let path_name = if variable == "DEEPKEEP_API_BASE" {
        "api_base"
    } else {
        "deepkeep_firewall_id"
    };
    let resolved = configured
        .filter(|text| !text.is_empty())
        .or_else(|| lookup.lookup(variable))
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: variable.to_owned(),
            path: format!("{prefix}.{path_name}"),
        })?;
    resolve_string(resolved, &format!("{prefix}.{path_name}"), lookup)
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
