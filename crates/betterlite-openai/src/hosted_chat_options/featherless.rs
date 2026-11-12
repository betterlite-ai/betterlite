use crate::openai_base_config::Headers;
use betterlite_config::sections::hosted_chat_settings::Settings as HostedSettings;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum HostedChatOptionsError {
    #[error("Featherless AI does not support the {name} parameter")]
    UnsupportedParam { name: String },
    #[error("Missing Featherless AI API Key")]
    MissingApiKey,
}
impl HostedChatOptionsError {
    // UnsupportedParamsError carries 400; the pinned missing-key check raises a bare ValueError with no status
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::UnsupportedParam { .. } => Some(400),
            Self::MissingApiKey => None,
        }
    }
}
pub const FEATHERLESS_DEFAULT_API_BASE: &str = "https://api.featherless.ai/v1";
pub const AIML_DEFAULT_API_BASE: &str = "https://api.aimlapi.com/v1";
pub const SUPPORTED_PARAMS: &[&str] = &[
    "stream",
    "frequency_penalty",
    "function_call",
    "functions",
    "logit_bias",
    "max_tokens",
    "max_completion_tokens",
    "n",
    "presence_penalty",
    "stop",
    "temperature",
    "top_p",
];
// B668's class snapshot: every constructor field starts absent and only a supplied non-null value is kept
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FeatherlessSettings {
    pub frequency_penalty: Option<Value>,
    pub function_call: Option<Value>,
    pub functions: Option<Value>,
    pub logit_bias: Option<Value>,
    pub max_tokens: Option<Value>,
    #[serde(rename = "n")]
    pub number: Option<Value>,
    pub presence_penalty: Option<Value>,
    pub stop: Option<Value>,
    pub temperature: Option<Value>,
    pub top_p: Option<Value>,
    pub response_format: Option<Value>,
    pub tool_choice: Option<Value>,
    pub tools: Option<Value>,
}
// LiteLLM drops rejected tool options only when the call argument or the loaded global is exactly true
#[derive(Debug, Clone, Copy)]
pub struct DropParams {
    pub argument: bool,
    pub global: bool,
}
pub fn map_openai_params(
    non_default: &IndexMap<String, Value>,
    existing: &mut IndexMap<String, Value>,
    drop: DropParams,
) -> Result<(), HostedChatOptionsError> {
    for (name, value) in non_default {
        match name.as_str() {
            "tool_choice" | "tools" => {
                if name == "tool_choice" && matches!(value.as_str(), Some("auto" | "none")) {
                    existing.insert(name.clone(), value.clone());
                } else if !(drop.argument || drop.global) {
                    return Err(HostedChatOptionsError::UnsupportedParam { name: name.clone() });
                }
            }
            "max_completion_tokens" => {
                existing.insert(String::from("max_tokens"), value.clone());
            }
            _ if SUPPORTED_PARAMS.contains(&name.as_str()) => {
                if !value.is_null() {
                    existing.insert(name.clone(), value.clone());
                }
            }
            _ => {}
        }
    }
    Ok(())
}
pub fn validate_environment(
    api_key: Option<&SecretString>,
    headers: Option<Headers>,
) -> Result<Headers, HostedChatOptionsError> {
    let key = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .ok_or(HostedChatOptionsError::MissingApiKey)?;
    let mut validated = headers.unwrap_or_default();
    validated.insert(
        "Authorization".to_owned(),
        format!("Bearer {}", key.expose_secret()),
    );
    validated.insert("Content-Type".to_owned(), "application/json".to_owned());
    Ok(validated)
}
// LiteLLM chains the explicit value, each environment alias, then the default, skipping an empty one at every step
fn resolved_identity(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    environment_base: Option<&str>,
    environment_key: Option<&SecretString>,
    default_base: &str,
) -> (String, Option<SecretString>) {
    let base = api_base
        .filter(|value| !value.is_empty())
        .or(environment_base)
        .unwrap_or(default_base)
        .to_owned();
    let key = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .or(environment_key)
        .cloned();
    (base, key)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatIdentity {
    Featherless,
    // AIML's chat identity reads only the primary key alias, never the image-side AIMLAPI_KEY
    Aiml,
}
#[must_use]
pub fn chat_identity(
    host: ChatIdentity,
    settings: &HostedSettings,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
) -> (String, Option<SecretString>) {
    match host {
        ChatIdentity::Featherless => resolved_identity(
            api_base,
            api_key,
            settings
                .featherless_ai_api_base
                .as_deref()
                .or(settings.featherless_api_base.as_deref()),
            settings
                .featherless_ai_api_key
                .as_ref()
                .or(settings.featherless_api_key.as_ref()),
            FEATHERLESS_DEFAULT_API_BASE,
        ),
        ChatIdentity::Aiml => resolved_identity(
            api_base,
            api_key,
            settings.aiml_api_base.as_deref(),
            settings.aiml_api_key.as_ref(),
            AIML_DEFAULT_API_BASE,
        ),
    }
}
