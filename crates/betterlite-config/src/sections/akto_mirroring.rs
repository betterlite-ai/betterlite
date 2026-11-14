use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::Error as DeserializeError;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

pub const DEFAULT_ACCOUNT_ID: &str = "1000000";
pub const DEFAULT_VXLAN_ID: &str = "0";
pub const DEFAULT_GUARDRAIL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    guardrails: Vec<Entry>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Entry {
    guardrail_name: Option<String>,
    litellm_params: Parameters,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Parameters {
    guardrail: String,
    akto_base_url: Option<String>,
    akto_api_key: Option<SecretString>,
    akto_account_id: Option<String>,
    akto_vxlan_id: Option<String>,
    unreachable_fallback: Option<String>,
    guardrail_timeout: Option<RawTimeout>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawTimeout {
    Integer(u64),
    Text(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
    pub api_base: String,
    pub api_key: SecretString,
    pub account_id: String,
    pub vxlan_id: String,
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
            if entry.litellm_params.guardrail == "akto" {
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
        let parameters = self.litellm_params;
        let base_path = format!("{prefix}.akto_base_url");
        let supplied_base = required_value(
            parameters.akto_base_url,
            "AKTO_GUARDRAIL_API_BASE",
            &base_path,
            lookup,
        )?;
        let api_base = supplied_base.trim_end_matches('/').to_owned();
        if api_base.is_empty() {
            return Err(ConfigError::MissingEnvironment {
                variable: "AKTO_GUARDRAIL_API_BASE".to_owned(),
                path: base_path,
            });
        }
        let api_key = SecretString::from(required_value(
            parameters
                .akto_api_key
                .map(|key| key.expose_secret().to_owned()),
            "AKTO_API_KEY",
            &format!("{prefix}.akto_api_key"),
            lookup,
        )?);
        let account_id = optional_value(
            parameters.akto_account_id,
            "AKTO_ACCOUNT_ID",
            DEFAULT_ACCOUNT_ID,
            &format!("{prefix}.akto_account_id"),
            lookup,
        )?;
        let vxlan_id = optional_value(
            parameters.akto_vxlan_id,
            "AKTO_VXLAN_ID",
            DEFAULT_VXLAN_ID,
            &format!("{prefix}.akto_vxlan_id"),
            lookup,
        )?;
        let unreachable_fallback = resolve_fallback(
            parameters.unreachable_fallback.as_deref(),
            &format!("{prefix}.unreachable_fallback"),
            lookup,
        )?;
        let timeout = resolve_timeout(
            parameters.guardrail_timeout,
            &format!("{prefix}.guardrail_timeout"),
            lookup,
        )?;
        Ok(Configured {
            guardrail_name: self.guardrail_name.unwrap_or_default(),
            api_base,
            api_key,
            account_id,
            vxlan_id,
            unreachable_fallback,
            timeout,
        })
    }
}

fn required_value<L>(
    configured: Option<String>,
    variable: &str,
    path: &str,
    lookup: &L,
) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let missing = || ConfigError::MissingEnvironment {
        variable: variable.to_owned(),
        path: path.to_owned(),
    };
    let candidate = configured
        .filter(|value| !value.is_empty())
        .or_else(|| lookup.lookup(variable))
        .ok_or_else(missing)?;
    let resolved = resolve_string(candidate, path, lookup)?;
    if resolved.is_empty() {
        return Err(missing());
    }
    Ok(resolved)
}

fn optional_value<L>(
    configured: Option<String>,
    variable: &str,
    default: &str,
    path: &str,
    lookup: &L,
) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let candidate = configured
        .filter(|value| !value.is_empty())
        .or_else(|| lookup.lookup(variable));
    let Some(text) = candidate else {
        return Ok(default.to_owned());
    };
    let resolved = resolve_string(text, path, lookup)?;
    if resolved.is_empty() {
        return Ok(default.to_owned());
    }
    Ok(resolved)
}

fn resolve_fallback<L>(
    configured: Option<&str>,
    path: &str,
    lookup: &L,
) -> Result<UnreachableFallback, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = configured else {
        return Ok(UnreachableFallback::default());
    };
    // an empty os.environ reference counts as unset, while an empty literal names the wrong value
    let is_reference = text.starts_with("os.environ/");
    let resolved = resolve_string(text.to_owned(), path, lookup)?;
    if resolved.is_empty() && is_reference {
        return Ok(UnreachableFallback::default());
    }
    match resolved.as_str() {
        "fail_closed" => Ok(UnreachableFallback::FailClosed),
        "fail_open" => Ok(UnreachableFallback::FailOpen),
        _ => Err(ConfigError::Yaml {
            source: <serde_saphyr::Error as DeserializeError>::custom(format!(
                "{path}: unreachable_fallback must be fail_closed or fail_open"
            )),
        }),
    }
}

fn resolve_timeout<L>(
    raw: Option<RawTimeout>,
    path: &str,
    lookup: &L,
) -> Result<Duration, ConfigError>
where
    L: EnvironmentLookup,
{
    let seconds = match raw {
        None => return Ok(DEFAULT_GUARDRAIL_TIMEOUT),
        Some(RawTimeout::Integer(number)) => number,
        Some(RawTimeout::Text(text)) => {
            let resolved = resolve_string(text, path, lookup)?;
            if resolved.is_empty() {
                return Ok(DEFAULT_GUARDRAIL_TIMEOUT);
            }
            resolved
                .trim()
                .parse::<u64>()
                .map_err(|source| ConfigError::InvalidInteger {
                    path: path.to_owned(),
                    source,
                })?
        }
    };
    Ok(Duration::from_secs(seconds))
}
