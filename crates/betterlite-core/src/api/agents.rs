pub mod collection;
pub mod item;

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::CallUpdate;
use betterlite_logging::logging_state::updates::update_from_kwargs;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

// LiteLLM hands back a provider config object here; core names the provider and its caller above builds that config
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentsProvider {
    Gemini,
}

// LiteLLM's merge_agent_headers: a static header wins every name collision, whatever the two sides' casing, and keeps its own
#[must_use]
pub fn merge_agent_headers(
    dynamic_headers: Option<&BTreeMap<String, String>>,
    static_headers: Option<&BTreeMap<String, String>>,
) -> Option<BTreeMap<String, String>> {
    let mut merged = dynamic_headers.cloned().unwrap_or_default();
    if let Some(static_map) = static_headers {
        let static_names: BTreeSet<String> =
            static_map.keys().map(|name| name.to_lowercase()).collect();
        merged.retain(|name, _value| !static_names.contains(&name.to_lowercase()));
        merged.extend(
            static_map
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
    }
    if merged.is_empty() {
        return None;
    }
    Some(merged)
}

// LiteLLM compares against LlmProviders.GEMINI.value, so the match is exact and case-sensitive
#[must_use]
pub fn get_provider_agents_api_config(custom_llm_provider: Option<&str>) -> Option<AgentsProvider> {
    match custom_llm_provider? {
        "gemini" => Some(AgentsProvider::Gemini),
        _ => None,
    }
}

// LiteLLM's _get_agents_api_config turns the provider without a native agents API into a BadRequestError
pub fn get_agents_api_config(custom_llm_provider: &str) -> Result<AgentsProvider, AgentsApiError> {
    get_provider_agents_api_config(Some(custom_llm_provider)).ok_or_else(|| {
        AgentsApiError::UnsupportedProvider {
            provider: custom_llm_provider.to_owned(),
        }
    })
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgentsApiError {
    #[error(
        "Provider '{provider}' does not have a native agents API. Use the proxy POST /v1/agents endpoint to store agents locally."
    )]
    UnsupportedProvider { provider: String },
}

impl AgentsApiError {
    // LiteLLM raises litellm.BadRequestError(message=..., model="", llm_provider=...) with no body, so type and param stay null
    #[must_use]
    pub fn class(&self) -> ErrorClass {
        let Self::UnsupportedProvider { provider } = self;
        let context = ErrorContext {
            message: self.to_string(),
            model: Some(String::new()),
            provider: Some(provider.clone()),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        };
        ErrorClass::BadRequest(Box::new(ResponseDetails::from(context)))
    }
}

// what LiteLLM's _make_logging_obj reads besides the logging object itself
pub struct AgentLoggingCall<'a> {
    pub kwargs: &'a IndexMap<String, Json>,
    pub model: &'a str,
    pub custom_llm_provider: &'a str,
    pub call_type: &'a str,
    pub optional_params: IndexMap<String, Json>,
}

// LiteLLM's _make_logging_obj: update_from_kwargs(model=..., optional_params=..., litellm_params={"litellm_call_id": ...}, custom_llm_provider=...)
pub fn make_logging_obj(
    logging: &mut LoggingState,
    call: AgentLoggingCall<'_>,
) -> Result<(), LoggingStateError> {
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        call.kwargs
            .get("litellm_call_id")
            .cloned()
            .unwrap_or(Json::Null),
    );
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(call.custom_llm_provider.to_owned()),
    );
    logging.call_type = call.call_type.to_owned();
    update_from_kwargs(
        logging,
        call.kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(call.model.to_owned()),
            user: None,
            optional_params: call.optional_params,
            additional_params,
        },
    )
}

// check names fix these test modules as api::agents::<check>, so their declarations live under tests/
include!("agents/tests/modules.rs");
