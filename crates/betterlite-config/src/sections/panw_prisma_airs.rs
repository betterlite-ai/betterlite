#[path = "panw_prisma_airs/tests.rs"]
mod tests;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};

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
    timeout: Option<RawNumber>,
    violation_message_template: Option<String>,
    default_on: Option<bool>,
    #[serde(rename = "experimental_use_latest_role_message_only")]
    latest_role_message_only: Option<bool>,
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
    pub masking: MaskingSettings,
    pub fallback_on_error: FallbackAction,
    pub timeout: f64,
    pub violation_message_template: Option<String>,
    pub default_on: bool,
    pub latest_role_message_only: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
pub struct MaskingSettings {
    pub mask_on_block: bool,
    pub content: ContentMaskSettings,
}

#[derive(Debug, Clone, Copy)]
pub struct ContentMaskSettings {
    pub request_content: bool,
    pub response_content: bool,
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
    let timeout = resolve_timeout(params.timeout, &prefix, lookup)?;
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
        masking: MaskingSettings {
            mask_on_block,
            content: ContentMaskSettings {
                request_content: mask_request_content,
                response_content: mask_response_content,
            },
        },
        fallback_on_error,
        timeout,
        violation_message_template: params.violation_message_template,
        default_on: params.default_on.unwrap_or(true),
        latest_role_message_only: params.latest_role_message_only,
    })
}

fn resolve_timeout<L>(
    timeout: Option<RawNumber>,
    prefix: &str,
    lookup: &L,
) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    Ok(
        resolve_number(timeout, &format!("{prefix}.timeout"), lookup, parse_float)?
            .unwrap_or(10.0_f64),
    )
}

fn resolve_api_key<L>(
    configured: Option<SecretString>,
    lookup: &L,
    prefix: &str,
) -> Result<SecretString, ConfigError>
where
    L: EnvironmentLookup,
{
    let raw_configured_text = configured.map(|secret| secret.expose_secret().to_owned());
    let resolved_configured_text =
        resolve_optional_env(raw_configured_text, "api_key", lookup, prefix)?;
    resolved_configured_text
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
