use std::collections::BTreeMap;

use betterlite_http::interactions::agents_delete_versions::AgentRequest as DeleteVersionsRequest;
use betterlite_http::interactions::base::{ClientError, Params};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::AgentItemRequest;
use super::error::{AgentItemError, AgentItemFailureError, ProviderFailure, failure};
use crate::api::agents::{AgentLoggingCall, get_agents_api_config, make_logging_obj};

// LiteLLM falls back to Gemini when neither the argument nor the kwargs name a provider
pub(crate) const DEFAULT_PROVIDER: &str = "gemini";

#[derive(Clone, Copy, Debug)]
pub(crate) enum Operation {
    Get,
    Delete,
    Versions,
}

impl Operation {
    pub(crate) const fn call_type(self) -> &'static str {
        match self {
            Self::Get => "get_agent",
            Self::Delete => "delete_agent",
            Self::Versions => "list_agent_versions",
        }
    }

    // the kwarg the async function leaves for the sync one, popped before GenericLiteLLMParams sees it
    pub(crate) const fn flag(self) -> &'static str {
        match self {
            Self::Get => "aget_agent",
            Self::Delete => "adelete_agent",
            Self::Versions => "alist_agent_versions",
        }
    }
}

pub(crate) struct Prepared {
    pub(crate) provider: String,
    pub(crate) params: Params,
}

// the steps LiteLLM runs before its HTTP handler: provider, litellm_params, logging metadata, then the support check
pub(crate) fn prepare<Provider>(
    request: &AgentItemRequest<'_>,
    operation: Operation,
    logging: &mut LoggingState,
    redactor: &Redactor,
) -> Result<Prepared, AgentItemFailureError<Provider>>
where
    Provider: ProviderFailure,
{
    let rejected = |error: AgentItemError<Provider>, provider: Option<&str>| {
        failure(error, request.name, provider, redactor)
    };
    let provider = match resolve_provider(request) {
        Ok(provider) => provider,
        Err(error) => return Err(rejected(AgentItemError::Client(Box::new(error)), None)),
    };
    let mut kwargs = request.kwargs.clone();
    let _flagged = kwargs.shift_remove(operation.flag());
    kwargs
        .entry("custom_llm_provider".to_owned())
        .or_insert_with(|| Json::Text(provider.clone()));
    let params: Params = kwargs
        .iter()
        .map(|(name, value)| (name.clone(), value_of(value)))
        .collect();
    let mut optional_params = IndexMap::new();
    optional_params.insert("name".to_owned(), Json::Text(request.name.to_owned()));
    let call = AgentLoggingCall {
        kwargs: &kwargs,
        model: request.name,
        custom_llm_provider: &provider,
        call_type: operation.call_type(),
        optional_params,
    };
    make_logging_obj(logging, call)
        .map_err(|error| rejected(AgentItemError::Logging(error), Some(provider.as_str())))?;
    get_agents_api_config(&provider)
        .map_err(|error| rejected(AgentItemError::Unsupported(error), Some(provider.as_str())))?;
    Ok(Prepared { provider, params })
}

// `custom_llm_provider or kwargs.get("custom_llm_provider") or "gemini"`, with the declared type checked
fn resolve_provider(request: &AgentItemRequest<'_>) -> Result<String, ClientError> {
    if let Some(provider) = request.custom_llm_provider {
        return Ok(provider.to_owned());
    }
    match request.kwargs.get("custom_llm_provider") {
        None | Some(Json::Null) => Ok(DEFAULT_PROVIDER.to_owned()),
        Some(Json::Text(provider)) => Ok(provider.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ClientError::FieldType {
                field: "custom_llm_provider",
                expected: "a string",
            })
        }
    }
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

// the named-agent request that delete and list_versions share
pub(crate) fn named_request<'request>(
    request: &'request AgentItemRequest<'_>,
    params: &'request Params,
) -> DeleteVersionsRequest<'request> {
    let extra_headers: BTreeMap<String, SecretString> = request
        .extra_headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect();
    DeleteVersionsRequest {
        name: request.name,
        litellm_params: params,
        extra_headers,
        timeout: request.timeout,
        request_timeout: request.request_timeout,
    }
}
