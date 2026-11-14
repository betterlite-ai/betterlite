use std::time::{Duration, TryFromFloatSecsError};

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::Error as DeserializeError;

pub const DEFAULT_API_BASE: &str = "https://api.dynamo.ai";
// LiteLLM leaves timeout unset, so the AsyncHTTPHandler default applies; its value is COMPLETION_HTTP_FALLBACK_SECONDS
pub const DEFAULT_GUARDRAIL_TIMEOUT: Duration = Duration::from_secs(600);
pub const API_KEY_VARIABLE: &str = "DYNAMOAI_API_KEY";
pub const API_BASE_VARIABLE: &str = "DYNAMOAI_API_BASE";
pub const POLICY_IDS_VARIABLE: &str = "DYNAMOAI_POLICY_IDS";
pub const MODEL_ID_VARIABLE: &str = "DYNAMOAI_MODEL_ID";

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
pub struct Parameters {
    guardrail: String,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    policy_ids: Option<Vec<String>>,
    model_id: Option<String>,
    timeout: Option<RawNumber>,
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
    pub policy_ids: Vec<String>,
    pub model_id: String,
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
            if entry.litellm_params.guardrail == "dynamoai" {
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
        self.litellm_params.resolve(
            &self.guardrail_name,
            &format!("guardrails.{index}.litellm_params"),
            lookup,
        )
    }
}

impl Parameters {
    pub fn resolve<L>(
        self,
        guardrail_name: &str,
        prefix: &str,
        lookup: &L,
    ) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let api_key = resolve_api_key(self.api_key, prefix, lookup)?;
        let api_base = match self.api_base.filter(|value| !value.is_empty()) {
            Some(value) => resolve_string(value, &format!("{prefix}.api_base"), lookup)?,
            None => lookup
                .lookup(API_BASE_VARIABLE)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        };
        let policy_ids = self.policy_ids.unwrap_or_else(|| env_policy_ids(lookup));
        let model_id = match self.model_id.filter(|value| !value.is_empty()) {
            Some(value) => resolve_string(value, &format!("{prefix}.model_id"), lookup)?,
            None => lookup.lookup(MODEL_ID_VARIABLE).unwrap_or_default(),
        };
        let timeout = match resolve_number(
            self.timeout,
            &format!("{prefix}.timeout"),
            lookup,
            parse_float,
        )? {
            Some(seconds) => Duration::try_from_secs_f64(seconds)
                .map_err(|source| timeout_error(&format!("{prefix}.timeout"), &source))?,
            None => DEFAULT_GUARDRAIL_TIMEOUT,
        };
        Ok(Configured {
            guardrail_name: guardrail_name.to_owned(),
            api_key,
            api_base,
            policy_ids,
            model_id,
            timeout,
        })
    }
}

fn timeout_error(path: &str, source: &TryFromFloatSecsError) -> ConfigError {
    ConfigError::Yaml {
        source: <serde_saphyr::Error as DeserializeError>::custom(format!("{path}: {source}")),
    }
}

// LiteLLM splits the variable on commas as it is, keeping any spaces around an id
fn env_policy_ids<L>(lookup: &L) -> Vec<String>
where
    L: EnvironmentLookup,
{
    match lookup.lookup(POLICY_IDS_VARIABLE) {
        Some(text) if !text.is_empty() => text.split(',').map(str::to_owned).collect(),
        Some(_) | None => Vec::new(),
    }
}

fn resolve_api_key<L>(
    api_key: Option<SecretString>,
    prefix: &str,
    lookup: &L,
) -> Result<SecretString, ConfigError>
where
    L: EnvironmentLookup,
{
    let key_text = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .map(|key| key.expose_secret().to_owned())
        .or_else(|| {
            lookup
                .lookup(API_KEY_VARIABLE)
                .filter(|key| !key.is_empty())
        })
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: API_KEY_VARIABLE.to_owned(),
            path: format!("{prefix}.api_key"),
        })?;
    Ok(SecretString::from(resolve_string(
        key_text,
        &format!("{prefix}.api_key"),
        lookup,
    )?))
}
