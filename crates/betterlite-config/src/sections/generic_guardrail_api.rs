use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

mod tests;

pub const UI_FRIENDLY_NAME: &str = "Generic Guardrail API";
const GUARDRAIL_PATH: &str = "/beta/litellm_basic_guardrail_api";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnreachableFallback {
    #[default]
    FailClosed,
    FailOpen,
}

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
    unreachable_fallback: Option<UnreachableFallback>,
    fail_on_error: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub guardrails: Vec<Configured>,
}

#[derive(Debug, Clone)]
pub struct Configured {
    pub guardrail_name: String,
    pub api_key: Option<SecretString>,
    pub api_base: String,
    pub unreachable_fallback: UnreachableFallback,
    pub fail_on_error: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            if entry.litellm_params.guardrail == "generic_guardrail_api" {
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
        let api_key = match parameters
            .api_key
            .filter(|key| !key.expose_secret().is_empty())
        {
            Some(key) => Some(SecretString::from(resolve_string(
                key.expose_secret().to_owned(),
                &format!("{prefix}.api_key"),
                lookup,
            )?)),
            None => lookup
                .lookup("GENERIC_GUARDRAIL_API_KEY")
                .map(SecretString::from),
        };
        let supplied_base = parameters
            .api_base
            .filter(|value| !value.is_empty())
            .or_else(|| lookup.lookup("GENERIC_GUARDRAIL_API_BASE"))
            .ok_or_else(|| ConfigError::MissingEnvironment {
                variable: "GENERIC_GUARDRAIL_API_BASE".to_owned(),
                path: format!("{prefix}.api_base"),
            })?;
        let resolved_base = resolve_string(supplied_base, &format!("{prefix}.api_base"), lookup)?;
        let trimmed_base = resolved_base.trim_end_matches('/');
        let api_base = if trimmed_base.ends_with(GUARDRAIL_PATH) {
            trimmed_base.to_owned()
        } else {
            format!("{trimmed_base}{GUARDRAIL_PATH}")
        };
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_key,
            api_base,
            unreachable_fallback: parameters.unreachable_fallback.unwrap_or_default(),
            fail_on_error: parameters.fail_on_error.unwrap_or(true),
        })
    }
}
