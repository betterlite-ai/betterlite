use betterlite_http::interactions::base::{ClientError, Params};
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_logging::logging_state::{LoggingState, LoggingStateError};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use super::InteractionGetRequest;
use super::error::{InteractionGetError, InteractionGetFailureError, failure};
use crate::api::agents::item::error::ProviderFailure;
use crate::api::interactions::utils::selection::{
    InteractionsProvider, select_interactions_provider,
};

// LiteLLM's `custom_llm_provider or "gemini"`: only an absent or null name takes the default
const DEFAULT_PROVIDER: &str = "gemini";
// the flag aget leaves for get, popped before either function builds litellm_params
const ASYNC_FLAG: &str = "aget_interaction";

pub(crate) struct Prepared {
    pub(crate) provider: String,
    pub(crate) client: ProviderId,
    pub(crate) params: Params,
}

pub(crate) fn prepare<Provider>(
    request: &InteractionGetRequest<'_>,
    logging: &mut LoggingState,
    redactor: &Redactor,
) -> Result<Prepared, InteractionGetFailureError<Provider>>
where
    Provider: ProviderFailure,
{
    let provider = match provider_name(request) {
        Ok(provider) => provider,
        Err(error) => {
            let fault = InteractionGetError::Client(Box::new(error));
            return Err(failure(fault, None, redactor));
        }
    };
    let Some(client) = client_for(&provider) else {
        let fault = InteractionGetError::Unsupported {
            provider: provider.clone(),
        };
        return Err(failure(fault, Some(provider.as_str()), redactor));
    };
    let mut kwargs = request.kwargs.clone();
    let _flag = kwargs.shift_remove(ASYNC_FLAG);
    kwargs
        .entry("custom_llm_provider".to_owned())
        .or_insert_with(|| Json::Text(provider.clone()));
    let params: Params = kwargs
        .iter()
        .map(|(name, value)| (name.clone(), json_value(value)))
        .collect();
    record_call(logging, &kwargs, request, &provider).map_err(|error| {
        failure(
            InteractionGetError::Logging(error),
            Some(provider.as_str()),
            redactor,
        )
    })?;
    Ok(Prepared {
        provider,
        client,
        params,
    })
}

// the config lookup is the support check LiteLLM runs, and its answer keys the async client
fn client_for(provider: &str) -> Option<ProviderId> {
    match select_interactions_provider(Some(provider)) {
        Some(InteractionsProvider::GoogleAiStudio) => Some(ProviderId::Gemini),
        Some(InteractionsProvider::VertexAi) if provider == "vertex_ai_beta" => {
            Some(ProviderId::VertexAiBeta)
        }
        Some(InteractionsProvider::VertexAi) => Some(ProviderId::VertexAi),
        None => None,
    }
}

// LiteLLM's update_from_kwargs call: the call id and the interaction id, then the merged metadata
fn record_call(
    logging: &mut LoggingState,
    kwargs: &IndexMap<String, Json>,
    request: &InteractionGetRequest<'_>,
    provider: &str,
) -> Result<(), LoggingStateError> {
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        request
            .kwargs
            .get("litellm_call_id")
            .cloned()
            .unwrap_or(Json::Null),
    );
    let mut optional_params = IndexMap::new();
    optional_params.insert(
        "interaction_id".to_owned(),
        Json::Text(request.interaction_id.to_owned()),
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

// `custom_llm_provider or kwargs.custom_llm_provider or "gemini"`, with the declared type checked
fn provider_name(request: &InteractionGetRequest<'_>) -> Result<String, ClientError> {
    let named = match request.custom_llm_provider {
        Some(provider) => Some(provider),
        None => match request.kwargs.get("custom_llm_provider") {
            None | Some(Json::Null) => None,
            Some(Json::Text(provider)) => Some(provider.as_str()),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(ClientError::FieldType {
                    field: "custom_llm_provider",
                    expected: "a string",
                });
            }
        },
    };
    Ok(named.map_or_else(|| DEFAULT_PROVIDER.to_owned(), str::to_owned))
}

fn json_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::Text(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(json_value).collect()),
        Json::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), json_value(item)))
                .collect(),
        ),
    }
}
