use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::response_input::{ResponsesInput, ResponsesItem, ResponsesTool};

use crate::api::responses_json::value_from_prompt;
use crate::api::responses_provider_selection::ParameterMap;

use super::records::FollowUpCall;

const STREAM: &str = "stream";
const TOOL_CHOICE: &str = "tool_choice";

pub struct RequestParamsInput<'request> {
    pub input: &'request ResponsesInput,
    pub model: &'request str,
    pub tools: Option<&'request [ResponsesTool]>,
    pub previous_response_id: Option<&'request str>,
    pub call_params: &'request ParameterMap,
    pub kwargs: &'request ParameterMap,
}

// core input/model/tools, then previous_response_id when present, then call_params, then kwargs; a later merge keeps an earlier key's position
#[must_use]
pub fn build_request_params(request: &RequestParamsInput<'_>) -> ParameterMap {
    let mut params = ParameterMap::new();
    params.insert(String::from("input"), input_value(request.input));
    params.insert(
        String::from("model"),
        JsonValue::Text(request.model.to_owned()),
    );
    params.insert(String::from("tools"), tools_value(request.tools));
    if let Some(id) = request.previous_response_id {
        params.insert(
            String::from("previous_response_id"),
            JsonValue::Text(id.to_owned()),
        );
    }
    for (name, value) in request.call_params {
        params.insert(name.clone(), value.clone());
    }
    for (name, value) in request.kwargs {
        params.insert(name.clone(), value.clone());
    }
    params
}

// dict(call_params) with stream forced off for an auto-executed first call (L1274-L1288)
#[must_use]
pub fn prepare_initial_call_params(
    call_params: &ParameterMap,
    should_auto_execute: bool,
) -> ParameterMap {
    let mut params = call_params.clone();
    if should_auto_execute {
        params.insert(STREAM.to_owned(), JsonValue::Bool(false));
    }
    params
}

// dict(call_params) with the caller's stream restored and tool_choice popped whatever it held (L1290-L1308)
#[must_use]
pub fn prepare_follow_up_call_params(
    call_params: &ParameterMap,
    original_stream: bool,
) -> ParameterMap {
    let mut params = call_params.clone();
    params.insert(STREAM.to_owned(), JsonValue::Bool(original_stream));
    let _removed = params.shift_remove(TOOL_CHOICE);
    params
}

// the pending _make_follow_up_call (L1103-L1118): absent tools and previous_response_id stay None
#[must_use]
pub fn follow_up_call(
    input: &[ResponsesItem],
    model: &str,
    tools: Option<&[ResponsesTool]>,
    previous_response_id: Option<&str>,
    kwargs: &ParameterMap,
) -> FollowUpCall {
    FollowUpCall {
        input: ResponsesInput::Items(input.to_vec()),
        model: model.to_owned(),
        tools: tools.map(<[ResponsesTool]>::to_vec),
        previous_response_id: previous_response_id.map(str::to_owned),
        kwargs: kwargs.clone(),
    }
}

fn input_value(input: &ResponsesInput) -> JsonValue {
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

fn tools_value(tools: Option<&[ResponsesTool]>) -> JsonValue {
    match tools {
        Some(tools) => JsonValue::Array(
            tools
                .iter()
                .map(|tool| value_from_prompt(&tool.0))
                .collect(),
        ),
        None => JsonValue::Null,
    }
}
