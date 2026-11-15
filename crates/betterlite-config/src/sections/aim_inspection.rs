use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::Section;
use super::guardrail_identity::item::GuardrailValue;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};

pub const DEFAULT_API_BASE: &str = "https://api.aim.security";
const ENV_PREFIX: &str = "os.environ/";
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
    inspect_embeddings: Option<GuardrailValue>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub guardrails: Vec<Configured>,
}

#[derive(Debug, Clone)]
pub struct Configured {
    pub guardrail_name: Option<String>,
    pub api_key: Option<SecretString>,
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

impl Entry {
    // LiteLLM builds each value as `param or os.environ[fallback] or default`, and the guardrail raises later when keyless
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("guardrails.{index}.litellm_params");
        let parameters = self.litellm_params;
        let api_key = resolve_secret(parameters.api_key, &format!("{prefix}.api_key"), lookup)?
            .filter(|key| !key.expose_secret().is_empty())
            .or_else(|| lookup.lookup("AIM_API_KEY").filter(|key| !key.is_empty()))
            .map(SecretString::from);
        let api_base = parameters
            .api_base
            .map(|value| resolve_string(value, &format!("{prefix}.api_base"), lookup))
            .transpose()?
            .filter(|value| !value.is_empty())
            .or_else(|| lookup.lookup("AIM_API_BASE"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_key,
            api_base,
            inspect_embeddings: embedding_flag(parameters.inspect_embeddings, &prefix, lookup)?,
        })
    }
}

// an os.environ reference is the one text form of the flag; any other value that is not a bool names the field
fn embedding_flag<L: EnvironmentLookup>(
    value: Option<GuardrailValue>,
    prefix: &str,
    lookup: &L,
) -> Result<bool, ConfigError> {
    let path = format!("{prefix}.inspect_embeddings");
    let flag = match value {
        None | Some(GuardrailValue::Null) => Some(false),
        Some(GuardrailValue::Boolean(flag)) => Some(flag),
        Some(GuardrailValue::Text(text)) => match text.strip_prefix(ENV_PREFIX) {
            Some(variable) => Some(
                betterlite_env::flag(variable, &lookup.lookup(variable).unwrap_or_default())?
                    .unwrap_or(false),
            ),
            None => None,
        },
        Some(_) => None,
    };
    flag.ok_or_else(|| ConfigError::InvalidScalar { path, value: false })
}
