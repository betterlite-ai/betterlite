use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

pub const DEFAULT_API_BASE: &str = "https://api.aim.security";
const GUARDRAIL_KIND: &str = "aim";

mod tests;

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
    api_key: Option<SecretString>,
    api_base: Option<String>,
    inspect_embeddings: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub guardrails: Vec<Configured>,
}

#[derive(Debug, Clone)]
pub struct Configured {
    pub guardrail_name: Option<String>,
    pub api_key: SecretString,
    pub api_base: String,
    pub inspect_embeddings: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            if entry.litellm_params.guardrail == GUARDRAIL_KIND {
                guardrails.push(entry.resolve(index, lookup)?);
            }
        }
        Ok(Settings { guardrails })
    }
}

fn configured_value<L>(value: Option<String>, variable: &str, lookup: &L) -> Option<String>
where
    L: EnvironmentLookup,
{
    value
        .filter(|value| !value.is_empty())
        .or_else(|| lookup.lookup(variable).filter(|value| !value.is_empty()))
}

impl Entry {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("guardrails.{index}.litellm_params");
        let parameters = self.litellm_params;
        let api_key = match configured_value(
            parameters.api_key.map(|key| key.expose_secret().to_owned()),
            "AIM_API_KEY",
            lookup,
        ) {
            Some(key) => {
                SecretString::from(resolve_string(key, &format!("{prefix}.api_key"), lookup)?)
            }
            None => {
                return Err(ConfigError::MissingEnvironment {
                    variable: "AIM_API_KEY".to_owned(),
                    path: format!("{prefix}.api_key"),
                });
            }
        };
        let api_base = match configured_value(parameters.api_base, "AIM_API_BASE", lookup) {
            Some(value) => resolve_string(value, &format!("{prefix}.api_base"), lookup)?,
            None => DEFAULT_API_BASE.to_owned(),
        };
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_key,
            api_base,
            inspect_embeddings: parameters.inspect_embeddings.unwrap_or(false),
        })
    }
}
