use std::collections::BTreeMap;
use std::fmt::Display;
use std::time::Duration;

use secrecy::SecretString;
use serde::Deserialize;
use serde::de::Error as DeserializeError;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::guardrail_identity::item::GuardrailValue;
use crate::sections::guardrail_params::mode::LitellmParamsMode;

mod resolve;
mod timeout;

pub const UI_FRIENDLY_NAME: &str = "EnkryptAI";
pub const DEFAULT_API_BASE: &str = "https://api.enkryptai.com";

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
    policy_name: Option<String>,
    deployment_name: Option<String>,
    detectors: Option<BTreeMap<String, GuardrailValue>>,
    block_on_violation: Option<bool>,
    mode: Option<LitellmParamsMode>,
    timeout: Option<timeout::Seconds>,
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
    pub policy_name: Option<String>,
    pub deployment_name: Option<String>,
    pub detectors: Option<BTreeMap<String, GuardrailValue>>,
    pub block_on_violation: bool,
    pub mode: Option<LitellmParamsMode>,
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
            if entry.litellm_params.guardrail == "enkryptai" {
                guardrails.push(entry.resolve(index, lookup)?);
            }
        }
        Ok(Settings { guardrails })
    }
}

fn yaml_error<M>(path: &str, message: M) -> ConfigError
where
    M: Display,
{
    ConfigError::Yaml {
        source: <serde_saphyr::Error as DeserializeError>::custom(format!("{path}: {message}")),
    }
}
