use std::time::Duration;

use betterlite_context::core_settings::CoreSettings;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use http::header::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::error::SkillsCallError;

pub const DEFAULT_SKILLS_PROVIDER: &str = "anthropic";
pub const LITELLM_PROXY_PROVIDER: &str = "litellm_proxy";

#[derive(Debug, Clone, Default)]
pub struct SkillsCallParams {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub extra_headers: HeaderMap,
    // LiteLLM accepts extra_query for every skill call but the get and delete paths never forward it
    pub extra_query: Option<IndexMap<String, Json>>,
    pub timeout: Option<Duration>,
    pub custom_llm_provider: Option<String>,
}

impl SkillsCallParams {
    #[must_use]
    pub fn provider(&self) -> &str {
        self.custom_llm_provider
            .as_deref()
            .unwrap_or(DEFAULT_SKILLS_PROVIDER)
    }
}

#[derive(Debug)]
pub struct SkillCall<'call> {
    pub skill_id: &'call str,
    pub params: SkillsCallParams,
    pub kwargs: &'call IndexMap<String, Json>,
    pub litellm_call_id: Option<&'call str>,
}

// one skill call together with the logging state its pre-call update writes into
#[derive(Debug)]
pub struct SkillRequest<'call> {
    pub call: &'call SkillCall<'call>,
    pub logging: &'call mut LoggingState,
}

// LiteLLM writes `timeout or request_timeout`, so an absent or zero timeout takes REQUEST_TIMEOUT
pub fn resolved_timeout(
    timeout: Option<Duration>,
    core: &CoreSettings,
) -> Result<Duration, SkillsCallError> {
    match timeout {
        Some(value) if !value.is_zero() => Ok(value),
        Some(_) | None => {
            Duration::try_from_secs_f64(core.request_timeout_seconds).map_err(|source| {
                SkillsCallError::Timeout {
                    seconds: core.request_timeout_seconds,
                    source,
                }
            })
        }
    }
}
