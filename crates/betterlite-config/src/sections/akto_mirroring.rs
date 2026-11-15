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
                guardrails.push(entry.litellm_params.resolve(
                    entry.guardrail_name,
                    &format!("guardrails.{index}.litellm_params"),
                    lookup,
                )?);
            }
        }
        Ok(Settings { guardrails })
    }
}

impl Parameters {
    fn resolve<L>(
        self,
        guardrail_name: Option<String>,
        prefix: &str,
        lookup: &L,
    ) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let (api_base, api_key) =
            resolve_connection(self.akto_base_url, self.akto_api_key, prefix, lookup)?;
        let account_id = resolve_value(
            self.akto_account_id,
            "AKTO_ACCOUNT_ID",
            Some(DEFAULT_ACCOUNT_ID),
            &format!("{prefix}.akto_account_id"),
            lookup,
        )?;
        let vxlan_id = resolve_value(
            self.akto_vxlan_id,
            "AKTO_VXLAN_ID",
            Some(DEFAULT_VXLAN_ID),
            &format!("{prefix}.akto_vxlan_id"),
            lookup,
        )?;
        let unreachable_fallback = resolve_fallback(
            self.unreachable_fallback.as_deref(),
            &format!("{prefix}.unreachable_fallback"),
            lookup,
        )?;
        let timeout = resolve_timeout(
            self.guardrail_timeout,
            &format!("{prefix}.guardrail_timeout"),
            lookup,
        )?;
        Ok(Configured {
            guardrail_name: guardrail_name.unwrap_or_default(),
            api_base,
            api_key,
            account_id,
            vxlan_id,
            unreachable_fallback,
            timeout,
        })
    }
}

// LiteLLM strips trailing slashes from the base URL and reads both values from the environment
fn resolve_connection<L>(
    base: Option<String>,
    key: Option<SecretString>,
    prefix: &str,
    lookup: &L,
) -> Result<(String, SecretString), ConfigError>
where
    L: EnvironmentLookup,
{
    let base_path = format!("{prefix}.akto_base_url");
    let supplied_base = resolve_value(base, "AKTO_GUARDRAIL_API_BASE", None, &base_path, lookup)?;
    let api_base = supplied_base.trim_end_matches('/').to_owned();
    if api_base.is_empty() {
        return Err(missing_environment("AKTO_GUARDRAIL_API_BASE", &base_path));
    }
    let key_text = key.map(|secret| secret.expose_secret().to_owned());
    let api_key = SecretString::from(resolve_value(
        key_text,
        "AKTO_API_KEY",
        None,
        &format!("{prefix}.akto_api_key"),
        lookup,
    )?);
    Ok((api_base, api_key))
}

fn missing_environment(variable: &str, path: &str) -> ConfigError {
    ConfigError::MissingEnvironment {
        variable: variable.to_owned(),
        path: path.to_owned(),
    }
}

fn resolve_value<L>(
    configured: Option<String>,
    variable: &str,
    default: Option<&str>,
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
        return default
            .map(str::to_owned)
            .ok_or_else(|| missing_environment(variable, path));
    };
    let resolved = resolve_string(text, path, lookup)?;
    if resolved.is_empty() {
        return default
            .map(str::to_owned)
            .ok_or_else(|| missing_environment(variable, path));
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
    match resolved.to_ascii_lowercase().as_str() {
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
