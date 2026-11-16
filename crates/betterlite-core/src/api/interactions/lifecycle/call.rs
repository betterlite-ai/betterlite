use std::error::Error as StdError;
use std::time::Duration;

use betterlite_http::interactions::base::Params;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use super::error::{InteractionLifecycleError, InteractionLifecycleFailure, failure};
use super::request::InteractionLifecycleRequest;
use crate::api::agents::item::error::ProviderFailure;
use crate::api::interactions::utils::selection::{
    InteractionsProvider, select_interactions_provider,
};

// LiteLLM's custom_llm_provider or "gemini": an absent or empty name means Gemini
pub(crate) const DEFAULT_PROVIDER: &str = "gemini";

#[derive(Clone, Copy, Debug)]
pub(crate) enum Operation {
    Delete,
    Cancel,
}

impl Operation {
    // the kwarg an async twin leaves for the sync function, popped before GenericLiteLLMParams sees it
    pub(crate) const fn flag(self) -> &'static str {
        match self {
            Self::Delete => "adelete_interaction",
            Self::Cancel => "acancel_interaction",
        }
    }
}

pub(crate) struct Prepared {
    pub(crate) provider: ProviderId,
    pub(crate) provider_name: String,
    pub(crate) params: Params,
    pub(crate) deadline: Duration,
}

// the steps LiteLLM runs before its HTTP handler: provider default, litellm_params, config selection, logging metadata
pub(crate) fn prepare<Provider, Settlement>(
    request: &InteractionLifecycleRequest<'_>,
    operation: Operation,
    logging: &mut LoggingState,
    redactor: &Redactor,
) -> Result<Prepared, InteractionLifecycleFailure<Provider, Settlement>>
where
    Provider: ProviderFailure,
    Settlement: StdError + Send + Sync + 'static,
{
    let provider_name = resolve_provider(request);
    let mut kwargs = request.kwargs.clone();
    let _flagged = kwargs.shift_remove(operation.flag());
    let params: Params = kwargs
        .iter()
        .map(|(name, value)| (name.clone(), value_of(value)))
        .collect();
    let Some(selected) = select_interactions_provider(Some(provider_name.as_str())) else {
        let unsupported = InteractionLifecycleError::Unsupported {
            provider: provider_name.clone(),
        };
        return Err(failure(unsupported, &provider_name, redactor));
    };
    let provider = provider_id(selected);
    record_call(logging, &kwargs, request.interaction_id, &provider_name).map_err(|error| {
        failure(
            InteractionLifecycleError::Logging(error),
            &provider_name,
            redactor,
        )
    })?;
    let deadline = resolve_deadline(request, &provider_name, redactor)?;
    Ok(Prepared {
        provider,
        provider_name,
        params,
        deadline,
    })
}

pub(crate) fn resolve_provider(request: &InteractionLifecycleRequest<'_>) -> String {
    match request.custom_llm_provider {
        Some(provider) if !provider.is_empty() => provider.to_owned(),
        Some(_) | None => DEFAULT_PROVIDER.to_owned(),
    }
}

// LiteLLM's timeout or litellm.request_timeout, whose seconds are a float it hands to httpx
fn resolve_deadline<Provider, Settlement>(
    request: &InteractionLifecycleRequest<'_>,
    provider: &str,
    redactor: &Redactor,
) -> Result<Duration, InteractionLifecycleFailure<Provider, Settlement>>
where
    Provider: ProviderFailure,
    Settlement: StdError + Send + Sync + 'static,
{
    if let Some(given) = request.timeout {
        return Ok(given);
    }
    let seconds = request.request_timeout.seconds;
    Duration::try_from_secs_f64(seconds).map_err(|source| {
        failure(
            InteractionLifecycleError::Timeout { seconds, source },
            provider,
            redactor,
        )
    })
}

// the async client cache keys on the provider, and both Vertex AI names share one client configuration
const fn provider_id(selected: InteractionsProvider) -> ProviderId {
    match selected {
        InteractionsProvider::GoogleAiStudio => ProviderId::Gemini,
        InteractionsProvider::VertexAi => ProviderId::VertexAi,
    }
}

// LiteLLM's update_from_kwargs(kwargs=..., model=None, optional_params={"interaction_id": ...}, litellm_params={"litellm_call_id": ...}, custom_llm_provider=...)
fn record_call(
    logging: &mut LoggingState,
    kwargs: &IndexMap<String, Json>,
    interaction_id: &str,
    provider: &str,
) -> Result<(), LoggingStateError> {
    let call_id = kwargs.get("litellm_call_id").cloned().unwrap_or(Json::Null);
    let mut litellm_params = IndexMap::new();
    litellm_params.insert("litellm_call_id".to_owned(), call_id);
    let mut optional_params = IndexMap::new();
    optional_params.insert(
        "interaction_id".to_owned(),
        Json::Text(interaction_id.to_owned()),
    );
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    update_from_kwargs(
        logging,
        kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )
}

fn value_of(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::Text(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(value_of).collect()),
        Json::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), value_of(item)))
                .collect(),
        ),
    }
}
