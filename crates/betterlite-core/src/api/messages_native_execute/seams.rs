use std::sync::Arc;

use async_trait::async_trait;
use betterlite_anthropic::tools::wire::ADVISOR_TYPE;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::entry::MessagesEntryResult;
use super::error::MessagesExecuteError;
use crate::api::messages_reasoning_route::seams::ProviderResolution;

// litellm/types/llms/anthropic.py's ANTHROPIC_ADVISOR_TOOL_TYPE
pub const ADVISOR_TOOL_TYPE: &str = ADVISOR_TYPE;
// litellm/constants.py's ADVISOR_NATIVE_PROVIDERS
pub const ADVISOR_NATIVE_PROVIDERS: [&str; 1] = ["anthropic"];

// AnthropicCacheControlHook.maybe_inject_cache_control's arguments; B4126 supplies the implementation
pub struct CacheControlRequest<'request> {
    pub messages: &'request [Json],
    pub system: Option<&'request Json>,
    pub kwargs: &'request mut IndexMap<String, Json>,
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub tools: Option<&'request [Json]>,
    pub api_base: Option<&'request str>,
}

// The (messages, system) pair the hook returns; either may have been replaced
pub struct CacheControlOutcome {
    pub messages: Vec<Json>,
    pub system: Option<Json>,
}

pub trait CacheControlHook: Send + Sync {
    fn maybe_inject_cache_control(
        &self,
        request: CacheControlRequest<'_>,
    ) -> Result<CacheControlOutcome, MessagesExecuteError>;
}

// CustomLogger.async_pre_request_hook: the hook map is mutable, and a returned map replaces it
#[async_trait]
pub trait PreRequestHook: Send + Sync {
    fn callback_name(&self) -> &str;

    async fn pre_request(
        &self,
        model: &str,
        messages: &[Json],
        kwargs: &mut IndexMap<String, Json>,
    ) -> Result<Option<IndexMap<String, Json>>, MessagesExecuteError>;
}

// The arguments a WebSearchInterceptionLogger's try_short_circuit_search receives
pub struct SearchRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [Json],
    pub tools: Option<&'request [Json]>,
    pub provider: Option<&'request str>,
    pub kwargs: &'request IndexMap<String, Json>,
}

// A WebSearchInterceptionLogger instance's try_short_circuit_search; the first non-null answer wins
#[async_trait]
pub trait SearchShortCircuit: Send + Sync {
    fn callback_name(&self) -> &str;

    async fn try_short_circuit_search(
        &self,
        request: &SearchRequest<'_>,
    ) -> Result<Option<Json>, MessagesExecuteError>;
}

// The keyword-only arguments anthropic_messages forwards to the first matching interceptor
pub struct InterceptorRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [Json],
    pub tools: Option<&'request [Json]>,
    pub stream: bool,
    pub max_tokens: i64,
    pub custom_llm_provider: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub metadata: Option<&'request Json>,
    pub stop_sequences: Option<&'request Json>,
    pub system: Option<&'request Json>,
    pub temperature: Option<&'request Json>,
    pub thinking: Option<&'request Json>,
    pub tool_choice: Option<&'request Json>,
    pub top_k: Option<&'request Json>,
    pub top_p: Option<&'request Json>,
    pub kwargs: &'request IndexMap<String, Json>,
}

// interceptors/base.py's MessagesInterceptor; get_messages_interceptors' list order decides
#[async_trait]
pub trait MessagesInterceptor: Send + Sync {
    fn can_handle(&self, tools: Option<&[Json]>, provider: Option<&str>) -> bool;

    async fn handle(
        &self,
        request: &InterceptorRequest<'_>,
    ) -> Result<MessagesEntryResult, MessagesExecuteError>;
}

// advisor.py's can_handle match, so an advisor request is refused before it reaches a provider
#[must_use]
pub fn advisor_requested(tools: Option<&[Json]>, provider: Option<&str>) -> bool {
    let Some(tool_list) = tools else {
        return false;
    };
    if tool_list.is_empty() {
        return false;
    }
    let has_advisor = tool_list.iter().any(|tool| {
        matches!(
            tool,
            Json::Object(fields)
                if matches!(fields.get("type"), Some(Json::Text(kind)) if kind.as_str() == ADVISOR_TOOL_TYPE)
        )
    });
    let is_non_native = !provider.is_some_and(|name| ADVISOR_NATIVE_PROVIDERS.contains(&name));
    has_advisor && is_non_native
}

// Everything the entry asks the composition layer for; each slice keeps the registry's order
pub struct MessagesEntrySeams<'seams> {
    pub cache: &'seams dyn CacheControlHook,
    pub provider_resolution: &'seams (dyn ProviderResolution + Send + Sync),
    pub pre_request_hooks: &'seams [Arc<dyn PreRequestHook>],
    pub search_short_circuits: &'seams [Arc<dyn SearchShortCircuit>],
    pub interceptors: &'seams [Arc<dyn MessagesInterceptor>],
    pub redactor: Arc<Redactor>,
}
