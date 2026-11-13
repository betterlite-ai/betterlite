use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use aws_smithy_eventstream::frame::{read_message_from, write_message_to};
use aws_smithy_types::event_stream::{Header, HeaderValue, Message};
use aws_smithy_types::str_bytes::StrBytes;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_server::guardrails::bedrock_passthrough::event_stream::hook::{
    PostCallSuccessHook, de_anonymize_event_stream,
};
use betterlite_server::guardrails::bedrock_passthrough::output::process_output_response;
use betterlite_server::guardrails::bedrock_passthrough::request::process_input_messages;
use betterlite_server::guardrails::bedrock_passthrough::routing::{
    EVENT_STREAM_MEDIA_TYPE, is_converse_endpoint, is_event_stream_content_type,
};
use betterlite_server::guardrails::bedrock_passthrough::text_paths::SkipFlags;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Routing(RoutingInput),
    InputTexts(InputTextsInput),
    OutputTexts(OutputTextsInput),
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
pub(crate) struct InputTextsInput {
    endpoint: String,
    data: BTreeMap<String, Value>,
    skip_system: bool,
    skip_tool: bool,
    returned_texts: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutputTextsInput {
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
            Input::Routing(input) => Ok(routing(input)),
            Input::InputTexts(input) => input_texts(input),
            Input::OutputTexts(input) => output_texts(input),
            Input::Stream(input) => stream(input),
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

fn input_texts(input: &InputTextsInput) -> HelperResult {
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
    block_on(process_input_messages(&mut data, &guardrail, skip, None))?;
    let (texts, model) = guardrail.seen();
    Ok(json!({
        "texts": texts,
        "model": model,
        "data": serde_json::to_value(&data).or_value_error()?,
    }))
}

fn output_texts(input: &OutputTextsInput) -> HelperResult {
    let mut response = guardrail(&input.response)?;
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
    block_on(process_output_response(
        &mut response,
        &guardrail,
        None,
        BTreeMap::new(),
        request_data.as_ref(),
    ))?;
    let (texts, model) = guardrail.seen();
    Ok(json!({
        "texts": texts,
        "model": model,
        "data": serde_json::to_value(&response).or_value_error()?,
    }))
}

fn stream(input: &StreamInput) -> HelperResult {
    let mut body = Vec::new();
    for frame in &input.frames {
        body.extend_from_slice(&encode_frame(frame)?);
    }
    body.extend_from_slice(&hex::decode(&input.trailing_hex).or_value_error()?);
    let hook = ScriptedHook {
        replace: input.replace.clone(),
    };
    let rewritten = block_on(de_anonymize_event_stream(
        &body,
        &hook,
        &BTreeMap::new(),
        &BTreeMap::new(),
    ))?;
    decode_stream(&rewritten)
}

fn encode_frame(frame: &FrameInput) -> Result<Vec<u8>, Box<HelperError>> {
    let payload = serde_json::to_vec(&frame.payload).or_value_error()?;
    let header = Header::new(
        ":event-type".to_owned(),
        HeaderValue::String(StrBytes::from(frame.event_type.clone())),
    );
    let message = Message::new(payload).add_header(header);
    let mut buffer = Vec::new();
    write_message_to(&message, &mut buffer).or_value_error()?;
    Ok(buffer)
}

fn decode_stream(body: &[u8]) -> HelperResult {
    let mut frames = Vec::new();
    let mut offset = 0_usize;
    while let Some(total) = frame_total_length(body, offset) {
        let end = offset.saturating_add(total);
        let raw = match body.get(offset..end) {
            Some(raw) => raw,
            None => break,
        };
        let message = read_message_from(raw).or_value_error()?;
        frames.push(frame_json(&message)?);
        offset = end;
    }
    let trailing = body.get(offset..).unwrap_or_default();
    Ok(json!({
        "frames": frames,
        "trailing": hex::encode(trailing),
    }))
}

fn frame_total_length(body: &[u8], offset: usize) -> Option<usize> {
    let (length_bytes, _rest) = body.get(offset..)?.split_first_chunk::<4>()?;
    let total = usize::try_from(u32::from_be_bytes(*length_bytes)).ok()?;
    if total < 16 {
        return None;
    }
    let end = offset.checked_add(total)?;
    (end <= body.len()).then_some(total)
}

fn frame_json(message: &Message) -> Result<Value, Box<HelperError>> {
    let mut headers = serde_json::Map::new();
    for header in message.headers() {
        headers.insert(
            header.name().as_str().to_owned(),
            header_value_json(header.value()),
        );
    }
    let payload = serde_json::from_slice::<Value>(message.payload().as_ref()).or_value_error()?;
    Ok(json!({ "headers": headers, "payload": payload }))
}

fn header_value_json(value: &HeaderValue) -> Value {
    match value.as_string() {
        Ok(text) => Value::String(text.as_str().to_owned()),
        Err(_) => Value::String(format!("{value:?}")),
    }
}

fn guardrail(value: &Value) -> Result<GuardrailValue, Box<HelperError>> {
    serde_json::from_slice(value.to_string().as_bytes()).or_value_error()
}

fn guardrail_map(
    value: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, GuardrailValue>, Box<HelperError>> {
    let mut converted = BTreeMap::new();
    for (key, entry) in value {
        converted.insert(key.clone(), guardrail(entry)?);
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

#[derive(Default)]
struct Seen {
    texts: Vec<String>,
    model: Option<String>,
}

struct ScriptedGuardrail {
    answer: Option<Vec<String>>,
    seen: Mutex<Seen>,
}

impl ScriptedGuardrail {
    fn new(answer: Option<Vec<String>>) -> Self {
        Self {
            answer,
            seen: Mutex::new(Seen::default()),
        }
    }

    fn record(&self, inputs: &GuardrailApiInputs) {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.texts = inputs.texts.clone().unwrap_or_default();
        seen.model = inputs.model.clone();
    }

    fn seen(&self) -> (Vec<String>, Option<String>) {
        let seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        (seen.texts.clone(), seen.model.clone())
    }
}

impl GuardrailContract for ScriptedGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        _request_data: &mut BTreeMap<String, GuardrailValue>,
        _input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        self.record(&inputs);
        let result = match &self.answer {
            Some(answer) => Ok(GuardrailApiInputs {
                texts: Some(answer.clone()),
                ..inputs
            }),
            None => Ok(inputs),
        };
        ready(result)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("scripted hook failure")]
struct HookFailure;

struct ScriptedHook {
    replace: BTreeMap<String, String>,
}

impl PostCallSuccessHook for ScriptedHook {
    type Error = HookFailure;

    fn post_call_success_hook(
        &self,
        _data: &BTreeMap<String, GuardrailValue>,
        _user_api_key_dict: &BTreeMap<String, GuardrailValue>,
        response: GuardrailValue,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Error>> + Send {
        ready(Ok(replace_blocks(&response, &self.replace)))
    }
}

fn replace_blocks(response: &GuardrailValue, replace: &BTreeMap<String, String>) -> GuardrailValue {
    let mut replaced = response.clone();
    if let Some(blocks) = content_blocks_mut(&mut replaced) {
        for entry in blocks.iter_mut() {
            let GuardrailValue::Object(block) = entry else {
                continue;
            };
            let Some(GuardrailValue::Text(text)) = block.get_mut("text") else {
                continue;
            };
            if let Some(replacement) = replace.get(text.as_str()) {
                text.clone_from(replacement);
            }
        }
    }
    replaced
}

fn content_blocks_mut(response: &mut GuardrailValue) -> Option<&mut Vec<GuardrailValue>> {
    let GuardrailValue::Object(fields) = response else {
        return None;
    };
    let Some(GuardrailValue::Object(output)) = fields.get_mut("output") else {
        return None;
    };
    let Some(GuardrailValue::Object(message)) = output.get_mut("message") else {
        return None;
    };
    match message.get_mut("content") {
        Some(GuardrailValue::Array(blocks)) => Some(blocks),
        _ => None,
    }
}
