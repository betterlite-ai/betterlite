#[path = "panw_prisma_airs/tests.rs"]
mod tests;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};

const DEFAULT_API_BASE: &str = "https://service.api.aisecurity.paloaltonetworks.com";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    guardrails: Vec<Entry>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Entry {
    #[serde(default)]
    guardrail_name: Option<String>,
    litellm_params: Parameters,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Parameters {
    guardrail: String,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    profile_name: Option<String>,
    app_name: Option<String>,
    mask_on_block: Option<bool>,
    mask_request_content: Option<bool>,
    mask_response_content: Option<bool>,
    fallback_on_error: Option<FallbackAction>,
    timeout: Option<f64>,
    violation_message_template: Option<String>,
    default_on: Option<bool>,
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
    pub profile_name: Option<String>,
    pub app_name: String,
    pub mask_on_block: bool,
    pub mask_request_content: bool,
    pub mask_response_content: bool,
    pub fallback_on_error: FallbackAction,
    pub timeout: f64,
    pub violation_message_template: Option<String>,
    pub default_on: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FallbackAction {
    Block,
    Allow,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            if entry.litellm_params.guardrail == "panw_prisma_airs" {
                guardrails.push(resolve(entry, index, lookup)?);
            }
        }
        Ok(Settings { guardrails })
    }
}

fn resolve<L>(entry: Entry, index: usize, lookup: &L) -> Result<Configured, ConfigError>
where
    L: EnvironmentLookup,
{
    let params = entry.litellm_params;
    let prefix = format!("guardrails.{index}.litellm_params");
    let api_key = resolve_api_key(params.api_key, lookup, &prefix)?;
    let api_base = resolve_optional_env(params.api_base, "api_base", lookup, &prefix)?
        .or_else(|| {
            lookup
                .lookup("PANW_PRISMA_AIRS_API_BASE")
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    let mask_on_block = params.mask_on_block.unwrap_or(false);
    let mask_request_content = params.mask_request_content.unwrap_or(false) || mask_on_block;
    let mask_response_content = params.mask_response_content.unwrap_or(false) || mask_on_block;
    let fallback_on_error = params.fallback_on_error.unwrap_or(FallbackAction::Block);
    Ok(Configured {
        guardrail_name: entry
            .guardrail_name
            .unwrap_or_else(|| "panw_prisma_airs".to_owned()),
        api_key,
        api_base,
        profile_name: params.profile_name,
        app_name: params
            .app_name
            .map_or_else(|| "LiteLLM".to_owned(), |name| format!("LiteLLM-{name}")),
        mask_on_block,
        mask_request_content,
        mask_response_content,
        fallback_on_error,
        timeout: params.timeout.unwrap_or(10.0),
        violation_message_template: params.violation_message_template,
        default_on: params.default_on.unwrap_or(true),
    })
}

fn resolve_api_key<L>(
    configured: Option<SecretString>,
    lookup: &L,
    prefix: &str,
) -> Result<SecretString, ConfigError>
where
    L: EnvironmentLookup,
{
    let configured_text = configured.map(|secret| secret.expose_secret().to_owned());
    let configured_text = resolve_optional_env(configured_text, "api_key", lookup, prefix)?;
    configured_text
        .or_else(|| {
            lookup
                .lookup("PANW_PRISMA_AIRS_API_KEY")
                .filter(|value| !value.is_empty())
        })
        .map(SecretString::from)
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: "PANW_PRISMA_AIRS_API_KEY".to_owned(),
            path: format!("{prefix}.api_key"),
        })
}

fn resolve_optional_env<L>(
    value: Option<String>,
    field: &str,
    lookup: &L,
    prefix: &str,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = value else {
        return Ok(None);
    };
    let Some(variable) = text.strip_prefix("os.environ/") else {
        return Ok(Some(text));
    };
    lookup
        .lookup(variable)
        .map(Some)
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: variable.to_owned(),
            path: format!("{prefix}.{field}"),
        })
}
