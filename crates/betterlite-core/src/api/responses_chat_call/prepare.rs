use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::api::codex_history_tools::hoist_additional_tools;
use crate::api::responses_chat_options::records::ChatCompletionRequest;
use crate::api::responses_chat_options::{
    RequestAssembly, RequestContext, chat_completion_request,
};

use super::error::PreparationError;
use super::records::{CompletionArguments, Prepared, ResponseCall, ResponseChatCall};

const SKIP_BRIDGE: &str = "_skip_responses_api_bridge";

// B2410 hoists additional_tools out of the input and B908 maps the bridged request, exactly once per call
pub fn prepare<Completions, Context>(
    deps: &ResponseChatCall<'_, Completions>,
    call: &mut ResponseCall<'_, Context>,
) -> Result<Prepared, PreparationError> {
    let (bridged, input) = hoisted_request(call)?;
    let request = encode(&bridged, "request")?;
    let (mapped, mapped_json) = mapped_request(deps, call, &bridged)?;
    Ok(Prepared {
        input,
        request,
        mapped,
        mapped_json,
    })
}

// The bridged request keeps LiteLLM's cleaned input and, only when something was hoisted, the extended tools
fn hoisted_request<Context>(
    call: &mut ResponseCall<'_, Context>,
) -> Result<(ResponsesApiRequestParams, Json), PreparationError> {
    let mut request_value = encode(call.request, "request")?;
    // ResponsesApiRequestParams always carries input, so the value is present after its own serialization
    let input = request_value.get("input").cloned().unwrap_or(Value::Null);
    let tools = tools_argument(request_value.get("tools"))?;
    let hoisted = hoist_additional_tools(&input, tools.as_deref());
    if let Some(message) = &hoisted.debug {
        tracing::debug!("{message}");
    }
    let bridged_input = hoisted.input;
    let Some(fields) = request_value.as_object_mut() else {
        return Err(PreparationError::FieldType {
            field: "request",
            expected: "an object",
        });
    };
    fields.insert("input".to_owned(), bridged_input.clone());
    if !hoisted.hoisted.is_empty() {
        fields.insert("tools".to_owned(), Value::Array(hoisted.tools));
    }
    let bridged: ResponsesApiRequestParams = decode(request_value, "request")?;
    let input: Json = decode(bridged_input, "input")?;
    Ok((bridged, input))
}

fn encode<T>(value: &T, field: &'static str) -> Result<Value, PreparationError>
where
    T: Serialize,
{
    serde_json::to_value(value).map_err(|source| PreparationError::Encode { field, source })
}

fn decode<T>(value: Value, field: &'static str) -> Result<T, PreparationError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value).map_err(|source| PreparationError::Encode { field, source })
}

fn mapped_request<Completions, Context>(
    deps: &ResponseChatCall<'_, Completions>,
    call: &mut ResponseCall<'_, Context>,
    bridged: &ResponsesApiRequestParams,
) -> Result<(ChatCompletionRequest, Map<String, Value>), PreparationError> {
    let mut assembly = RequestAssembly {
        limits: deps.limits,
        cache: deps.cache,
        parameters: deps.parameters,
        probe: deps.probe,
        logging: call.logging.as_deref_mut(),
        route_all_openai_to_responses: deps.route_all_openai_to_responses,
    };
    let context = RequestContext {
        request: bridged,
        provider: call.custom_llm_provider,
        stream: call.stream,
        extra_headers: call.extra_headers,
        metadata: call.kwargs.get("metadata"),
        service_tier: text_argument(call.kwargs, "service_tier")?,
        api_base: text_argument(call.kwargs, "api_base")?,
    };
    let mapped = chat_completion_request(&context, &mut assembly)?;
    let mapped_value = encode(&mapped, "mapped")?;
    let Value::Object(mapped_json) = mapped_value else {
        return Err(PreparationError::FieldType {
            field: "mapped",
            expected: "an object",
        });
    };
    Ok((mapped, mapped_json))
}

fn tools_argument(tools: Option<&Value>) -> Result<Option<Vec<Value>>, PreparationError> {
    match tools {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => Ok(Some(items.clone())),
        Some(Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_)) => {
            Err(PreparationError::FieldType {
                field: "tools",
                expected: "a list",
            })
        }
    }
}

// LiteLLM copies the caller's kwargs first and lays the mapped request over them, so the mapped value wins
pub fn arguments(
    kwargs: &IndexMap<String, Json>,
    mapped: &Map<String, Value>,
) -> Result<CompletionArguments, PreparationError> {
    let mut fields = IndexMap::with_capacity(kwargs.len().saturating_add(mapped.len()));
    for (name, value) in kwargs {
        let encoded = serde_json::to_value(value).map_err(|source| PreparationError::Encode {
            field: "arguments",
            source,
        })?;
        fields.insert(name.clone(), encoded);
    }
    for (name, value) in mapped {
        fields.insert(name.clone(), value.clone());
    }
    fields.insert(SKIP_BRIDGE.to_owned(), Value::Bool(true));
    Ok(CompletionArguments { fields })
}

pub fn litellm_metadata(
    kwargs: &IndexMap<String, Json>,
) -> Result<Option<IndexMap<String, Json>>, PreparationError> {
    match kwargs.get("litellm_metadata") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(PreparationError::FieldType {
                field: "litellm_metadata",
                expected: "an object",
            })
        }
    }
}

fn text_argument<'args>(
    kwargs: &'args IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<&'args str>, PreparationError> {
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(PreparationError::FieldType {
                field,
                expected: "text",
            })
        }
    }
}
