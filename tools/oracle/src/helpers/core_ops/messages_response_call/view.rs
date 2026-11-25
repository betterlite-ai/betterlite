use std::str::from_utf8;

use betterlite_core::api::messages_chat_output::output::MessageResponse;
use betterlite_core::api::responses_create::call::ResponsesCreateCall;
use betterlite_core::api::responses_create_http::config::Headers;
use betterlite_core::api::responses_provider_selection::ParameterMap;
use betterlite_models::decisions::Json;
use betterlite_openai::response_input::{ResponsesInput, ResponsesTool};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::Object;
use serde::Serialize;

// The dict _build_responses_kwargs hands litellm.aresponses: the named arguments bind by name and every remaining key is **kwargs.
#[derive(Serialize)]
pub(super) struct CallView<'call> {
    model: &'call str,
    input: &'call ResponsesInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    include: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<&'call str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<&'call str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    background: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_format: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'call Vec<ResponsesTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncation: Option<&'call str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<&'call str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<&'call str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    safety_identifier: Option<&'call str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra_headers: Option<&'call Headers>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra_query: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra_body: Option<&'call Object>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_openai_params: Option<&'call Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_llm_provider: Option<&'call str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_base: Option<&'call str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ssl_verify: Option<bool>,
    // api_key is encrypted at rest and shared_session has no Python counterpart, so no case forwards either
    #[serde(flatten)]
    kwargs: &'call ParameterMap,
}

impl<'call> CallView<'call> {
    pub(super) fn new(call: &'call ResponsesCreateCall) -> Self {
        Self {
            model: &call.model,
            input: &call.input,
            include: call.include.as_ref(),
            instructions: call.instructions.as_deref(),
            max_output_tokens: call.max_output_tokens,
            prompt: call.prompt.as_ref(),
            metadata: call.metadata.as_ref(),
            parallel_tool_calls: call.parallel_tool_calls,
            previous_response_id: call.previous_response_id.as_deref(),
            reasoning: call.reasoning.as_ref(),
            store: call.store,
            background: call.background,
            stream: call.stream,
            temperature: call.temperature,
            text: call.text.as_ref(),
            text_format: call.text_format.as_ref(),
            tool_choice: call.tool_choice.as_ref(),
            tools: call.tools.as_ref(),
            top_p: call.top_p,
            truncation: call.truncation.as_deref(),
            user: call.user.as_deref(),
            service_tier: call.service_tier.as_deref(),
            safety_identifier: call.safety_identifier.as_deref(),
            extra_headers: call.extra_headers.as_ref(),
            extra_query: call.extra_query.as_ref(),
            extra_body: call.extra_body.as_ref(),
            timeout: call.timeout,
            allowed_openai_params: call.allowed_openai_params.as_ref(),
            custom_llm_provider: call.custom_llm_provider.as_deref(),
            api_base: call.api_base.as_deref(),
            ssl_verify: call.ssl_verify,
            kwargs: &call.kwargs,
        }
    }
}

// The handler's two answers: the translated message or the wrapper's SSE frames, each beside the call that produced it.
#[derive(Serialize)]
pub(super) struct Output {
    pub(super) request: Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) native: Option<MessageResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) frames: Option<Vec<Frame>>,
}

// One SSE frame as the wire contract fixes it: the event name and the data line's decoded JSON.
#[derive(Serialize)]
pub(super) struct Frame {
    pub(super) event: String,
    pub(super) data: Json,
}

// The frame's two lines: an "event: " name and a "data: " JSON line. Key order and escapes are no part of the wire contract.
pub(super) fn frame_value(raw: &[u8]) -> Result<Frame, Box<HelperError>> {
    let text = from_utf8(raw).map_err(|error| invalid_frame(&error.to_string()))?;
    let body = text
        .strip_suffix("\n\n")
        .ok_or_else(|| invalid_frame(text))?;
    let mut lines = body.split('\n');
    let event_line = lines.next().and_then(|line| line.strip_prefix("event: "));
    let data_line = lines.next().and_then(|line| line.strip_prefix("data: "));
    let (Some(name), Some(payload), None) = (event_line, data_line, lines.next()) else {
        return Err(invalid_frame(text));
    };
    let parsed =
        serde_json::from_str::<Json>(payload).map_err(|error| invalid_frame(&error.to_string()))?;
    Ok(Frame {
        event: name.to_owned(),
        data: parsed,
    })
}

fn invalid_frame(text: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: format!("not an SSE frame: {text}"),
    })
}
