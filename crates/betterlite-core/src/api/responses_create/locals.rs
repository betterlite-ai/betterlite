use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::headers::affinity::SessionMetadata;
use betterlite_openai::response_input::ResponsesInput;
use serde_json::Number;

use crate::api::responses_create_http::config::Headers;
use crate::api::responses_provider_selection::ParameterMap;

use super::call::ResponsesCreateCall;
use super::convert::{json_value, object_value, reencode, value_from_prompt};
use super::error::CreateError;

// locals() as responses() sees it: every named argument, nulls included, under the resolved model and provider
pub(super) fn named_locals(
    call: &ResponsesCreateCall,
    model: &str,
    provider: Option<&str>,
    text: &Option<Json>,
    extra_headers: &Option<Headers>,
) -> ParameterMap {
    let mut values = ParameterMap::new();
    values.insert(String::from("input"), input_value(&call.input));
    values.insert(String::from("model"), JsonValue::Text(model.to_owned()));
    values.insert(
        String::from("custom_llm_provider"),
        provider.map_or(JsonValue::Null, |name| JsonValue::Text(name.to_owned())),
    );
    insert(
        &mut values,
        "include",
        call.include.as_ref().map(json_value),
    );
    insert(
        &mut values,
        "instructions",
        call.instructions.clone().map(JsonValue::Text),
    );
    insert(
        &mut values,
        "max_output_tokens",
        call.max_output_tokens
            .map(|count| JsonValue::Number(count.into())),
    );
    insert(&mut values, "prompt", call.prompt.as_ref().map(json_value));
    insert(
        &mut values,
        "metadata",
        call.metadata.as_ref().map(json_value),
    );
    insert(
        &mut values,
        "parallel_tool_calls",
        call.parallel_tool_calls.map(JsonValue::Bool),
    );
    insert(
        &mut values,
        "previous_response_id",
        call.previous_response_id.clone().map(JsonValue::Text),
    );
    insert(
        &mut values,
        "reasoning",
        call.reasoning.as_ref().map(json_value),
    );
    insert(&mut values, "store", call.store.map(JsonValue::Bool));
    insert(
        &mut values,
        "background",
        call.background.map(JsonValue::Bool),
    );
    insert(&mut values, "stream", call.stream.map(JsonValue::Bool));
    insert(
        &mut values,
        "temperature",
        call.temperature.and_then(number),
    );
    insert(&mut values, "text", text.as_ref().map(json_value));
    insert(
        &mut values,
        "text_format",
        call.text_format.as_ref().map(json_value),
    );
    insert(
        &mut values,
        "tool_choice",
        call.tool_choice.as_ref().map(json_value),
    );
    insert(
        &mut values,
        "tools",
        call.tools.as_ref().map(|tools| {
            JsonValue::Array(
                tools
                    .iter()
                    .map(|tool| value_from_prompt(&tool.0))
                    .collect(),
            )
        }),
    );
    insert(&mut values, "top_p", call.top_p.and_then(number));
    insert(
        &mut values,
        "truncation",
        call.truncation.clone().map(JsonValue::Text),
    );
    insert(&mut values, "user", call.user.clone().map(JsonValue::Text));
    insert(
        &mut values,
        "service_tier",
        call.service_tier.clone().map(JsonValue::Text),
    );
    insert(
        &mut values,
        "safety_identifier",
        call.safety_identifier.clone().map(JsonValue::Text),
    );
    insert(
        &mut values,
        "extra_headers",
        extra_headers.as_ref().map(header_value),
    );
    insert(
        &mut values,
        "extra_query",
        call.extra_query.as_ref().map(json_value),
    );
    insert(
        &mut values,
        "extra_body",
        call.extra_body.as_ref().map(object_value),
    );
    insert(&mut values, "timeout", call.timeout.and_then(number));
    insert(
        &mut values,
        "allowed_openai_params",
        call.allowed_openai_params
            .as_ref()
            .map(|names| JsonValue::Array(names.iter().cloned().map(JsonValue::Text).collect())),
    );
    values
}

pub(super) fn insert(values: &mut ParameterMap, name: &str, value: Option<JsonValue>) {
    values.insert(name.to_owned(), value.unwrap_or(JsonValue::Null));
}

fn number(value: f64) -> Option<JsonValue> {
    Number::from_f64(value).map(JsonValue::Number)
}

fn header_value(headers: &Headers) -> JsonValue {
    JsonValue::Object(
        headers
            .iter()
            .map(|(name, value)| (name.clone(), JsonValue::Text(value.clone())))
            .collect(),
    )
}

pub(super) fn text_param(
    kwargs: &ParameterMap,
    name: &'static str,
) -> Result<Option<String>, CreateError> {
    match kwargs.get(name) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Text(text)) => Ok(Some(text.clone())),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => Err(CreateError::Field {
            field: name,
            expected: "a string",
        }),
    }
}

pub(super) fn session_metadata(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<SessionMetadata>, CreateError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(_)) => reencode(value).map(Some).map_err(CreateError::Encode),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(CreateError::Field {
                field,
                expected: "an object",
            })
        }
    }
}

pub(super) fn input_value(input: &ResponsesInput) -> JsonValue {
    match input {
        ResponsesInput::Text(text) => JsonValue::Text(text.clone()),
        ResponsesInput::Items(items) => JsonValue::Array(
            items
                .iter()
                .map(|item| value_from_prompt(&item.0))
                .collect(),
        ),
    }
}
