use std::fmt::{self, Formatter};
use std::sync::Arc;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::openai_base_config::Object;
use betterlite_prompts::arguments::json::Object as PromptsObject;

use crate::aws_settings::credential_cache::CredentialCache;
use crate::bedrock_model_capabilities::mantle::MANTLE_MESSAGES_PATH;
use crate::mantle_inputs::records::MantleGlobals;
use crate::mantle_inputs::region::{region_from_host, resolve_region};
use crate::messages::config::MessagesConfig;

use super::error::MantleSettingsError;
use super::field_text;

pub const CUSTOM_LLM_PROVIDER: ProviderId = ProviderId::BedrockMantle;
pub const USES_GET_LLM_PROVIDER_API_BASE: bool = true;
const CANONICAL_HOST: &str = "https://bedrock-mantle";
const BASE_SUFFIXES: [&str; 6] = [
    MANTLE_MESSAGES_PATH,
    "/v1/messages",
    "/messages",
    "/anthropic/v1",
    "/openai/v1",
    "/v1",
];

// BedrockMantleAnthropicMessagesConfig.__init__ runs the inherited B622 constructor, which rejects keyword arguments, then keeps a truthy injected signer or points back at this config.
pub struct MantleMessagesConfig {
    pub base: MessagesConfig,
    pub credential_cache: Option<Arc<CredentialCache>>,
}

impl fmt::Debug for MantleMessagesConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("MantleMessagesConfig")
            .field("base", &self.base)
            .field("has_credential_cache", &self.credential_cache.is_some())
            .finish()
    }
}

impl MantleMessagesConfig {
    pub fn new(
        kwargs: &PromptsObject,
        credential_cache: Option<Arc<CredentialCache>>,
    ) -> Result<Self, MantleSettingsError> {
        let base = MessagesConfig::try_from(kwargs)?;
        Ok(Self {
            base,
            credential_cache,
        })
    }
}

// build_mantle_native_messages_url: the region chain reads litellm_params with api_base overwritten even when None, the base comes from the explicit argument, the env base or the canonical host, only the first matching suffix is stripped, and a recognized Mantle host is normalized to its canonical form.
pub fn build_native_messages_url(
    api_base: Option<&str>,
    litellm_params: &Object,
    globals: &MantleGlobals,
) -> Result<String, MantleSettingsError> {
    let explicit = field_text(litellm_params, "aws_region_name")?;
    let region = resolve_region(explicit.as_deref(), api_base, globals)?;
    let chosen = api_base
        .filter(|value| !value.is_empty())
        .or_else(|| {
            globals
                .mantle_api_base
                .as_deref()
                .filter(|value| !value.is_empty())
        })
        .map_or_else(
            || format!("{CANONICAL_HOST}.{region}.api.aws"),
            str::to_owned,
        );
    let configured = chosen.trim_end_matches('/');
    let stripped = BASE_SUFFIXES
        .iter()
        .find_map(|suffix| configured.strip_suffix(suffix))
        .unwrap_or(configured);
    let host = match region_from_host(stripped) {
        Some(_) => format!("{CANONICAL_HOST}.{region}.api.aws"),
        None => stripped.to_owned(),
    };
    Ok(format!("{host}{MANTLE_MESSAGES_PATH}"))
}
