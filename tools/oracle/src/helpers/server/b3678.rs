use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_server::guardrails::bedrock_passthrough::event_stream::hook::de_anonymize_event_stream;
use betterlite_server::guardrails::bedrock_passthrough::output::process_output_response;
use betterlite_server::guardrails::bedrock_passthrough::request::process_input_messages;
use betterlite_server::guardrails::bedrock_passthrough::routing::{
    EVENT_STREAM_MEDIA_TYPE, is_converse_endpoint, is_event_stream_content_type,
};
use betterlite_server::guardrails::bedrock_passthrough::text_paths::SkipFlags;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod frames;
mod scripted;

use frames::{decode_stream, encode_stream};
use scripted::{ScriptedGuardrail, ScriptedHook};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Routing(RoutingInput),
    #[serde(rename = "input_texts")]
    Request(TextsRequest),
    #[serde(rename = "output_texts")]
    Response(TextsResponse),
    Stream(StreamInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoutingInput {
    endpoint: String,
    content_type: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TextsRequest {
    endpoint: String,
    data: BTreeMap<String, Value>,
    skip_system: bool,
    skip_tool: bool,
    returned_texts: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TextsResponse {
    endpoint: String,
    response: Value,
    request_data: Option<BTreeMap<String, Value>>,
    returned_texts: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamInput {
    frames: Vec<FrameInput>,
    replace: BTreeMap<String, String>,
    trailing_hex: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FrameInput {
    event_type: String,
    payload: Value,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Routing(routing_input) => Ok(routing(routing_input)),
            Input::Request(texts_request) => input_texts(texts_request),
            Input::Response(texts_response) => output_texts(texts_response),
            Input::Stream(stream_input) => stream(stream_input),
        }
    }
}

fn routing(input: &RoutingInput) -> Value {
    json!({
        "is_converse": is_converse_endpoint(&input.endpoint),
        "is_event_stream": is_event_stream_content_type(&input.content_type),
        "media_type": EVENT_STREAM_MEDIA_TYPE,
    })
}

fn input_texts(input: &TextsRequest) -> HelperResult {
    let mut data = guardrail_map(&input.data)?;
    data.insert(
        "endpoint".to_owned(),
        GuardrailValue::Text(input.endpoint.clone()),
    );
    let guardrail = ScriptedGuardrail::new(input.returned_texts.clone());
    let skip = SkipFlags {
        system: input.skip_system,
        tool: input.skip_tool,
    };
    let outcome = block_on(process_input_messages(&mut data, &guardrail, skip, None))?;
    outcome.or_value_error()?;
    let seen = guardrail.seen();
    Ok(json!({
        "texts": seen.texts,
        "model": seen.model,
        "data": serde_json::to_value(&data).or_value_error()?,
    }))
}

fn output_texts(input: &TextsResponse) -> HelperResult {
    let response_text = input.response.to_string();
    let mut response =
        serde_json::from_slice::<GuardrailValue>(response_text.as_bytes()).or_value_error()?;
    let request_data = match &input.request_data {
        Some(value) => {
            let mut request_data = guardrail_map(value)?;
            request_data.insert(
                "endpoint".to_owned(),
                GuardrailValue::Text(input.endpoint.clone()),
            );
            Some(request_data)
        }
        None => None,
    };
    let guardrail = ScriptedGuardrail::new(input.returned_texts.clone());
    let outcome = block_on(process_output_response(
        &mut response,
        &guardrail,
        None,
        BTreeMap::new(),
        request_data.as_ref(),
    ))?;
    outcome.or_value_error()?;
    let seen = guardrail.seen();
    Ok(json!({
        "texts": seen.texts,
        "model": seen.model,
        "data": serde_json::to_value(&response).or_value_error()?,
    }))
}

fn stream(input: &StreamInput) -> HelperResult {
    let body = encode_stream(&input.frames, &input.trailing_hex)?;
    let hook = ScriptedHook {
        replace: input.replace.clone(),
    };
    let outcome = block_on(de_anonymize_event_stream(
        &body,
        &hook,
        &BTreeMap::new(),
        &BTreeMap::new(),
    ))?;
    let rewritten = outcome.or_value_error()?;
    decode_stream(&rewritten)
}

fn guardrail_map(
    value: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, GuardrailValue>, Box<HelperError>> {
    let mut converted = BTreeMap::new();
    for (key, entry) in value {
        let text = entry.to_string();
        converted.insert(
            key.clone(),
            serde_json::from_slice(text.as_bytes()).or_value_error()?,
        );
    }
    Ok(converted)
}

fn block_on<F, T>(future: F) -> Result<T, Box<HelperError>>
where
    F: Future<Output = T>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .or_value_error()?;
    Ok(runtime.block_on(future))
}
