use std::collections::BTreeMap;
use std::str;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::guardrails::anthropic_translation::sse_events::SseChunk;
use betterlite_server::guardrails::anthropic_translation::stream_rewrite::ended::{
    rewrite_ended_stream_text, rewrite_ended_tool_calls,
};
use betterlite_server::guardrails::anthropic_translation::usage_response::build_streaming_usage_response;
use betterlite_server::guardrails::tool_call_identity::ToolCallShape;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct BytesChunk {
    bytes: String,
}

#[derive(Deserialize)]
pub(crate) struct EventChunk {
    event: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum ChunkInput {
    Bytes(BytesChunk),
    Event(EventChunk),
}

#[derive(Deserialize)]
pub(crate) struct ShapeInput {
    name: Option<String>,
    arguments: String,
}

#[derive(Deserialize)]
pub(crate) struct TextScenario {
    responses_so_far: Vec<ChunkInput>,
    rewritten_text: String,
    guardrail_name: String,
}

#[derive(Deserialize)]
pub(crate) struct ToolScenario {
    responses_so_far: Vec<ChunkInput>,
    pre_guardrail_tool_calls: Vec<ShapeInput>,
    post_guardrail_tool_calls: Vec<ShapeInput>,
    guardrail_name: String,
}

#[derive(Deserialize)]
pub(crate) struct UsageScenario {
    responses_so_far: Vec<ChunkInput>,
    request_data: Option<Map<String, Value>>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Function {
    TextRewrite { scenarios: Vec<TextScenario> },
    ToolRewrite { scenarios: Vec<ToolScenario> },
    UsageResponse { scenarios: Vec<UsageScenario> },
}

#[derive(Deserialize, Serialize)]
pub(crate) struct ProjectedMessage {
    role: String,
    content: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct ProjectedChoice {
    finish_reason: Option<String>,
    message: ProjectedMessage,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct ProjectedUsage {
    #[serde(rename = "prompt_tokens")]
    prompt: i64,
    #[serde(rename = "completion_tokens")]
    completion: i64,
    #[serde(rename = "total_tokens")]
    total: i64,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct ProjectedResponse {
    object: String,
    model: String,
    choices: Vec<ProjectedChoice>,
    usage: ProjectedUsage,
}

fn chunk_of(input: &ChunkInput) -> SseChunk {
    match input {
        ChunkInput::Bytes(chunk) => SseChunk::Bytes(chunk.bytes.clone().into_bytes()),
        ChunkInput::Event(chunk) => SseChunk::Event(chunk.event.clone()),
    }
}

fn shapes(input: &[ShapeInput]) -> Vec<ToolCallShape> {
    input
        .iter()
        .map(|shape| ToolCallShape {
            name: shape.name.clone(),
            arguments: shape.arguments.clone(),
        })
        .collect()
}

fn encode_chunks(chunks: &[SseChunk]) -> HelperResult {
    let values = chunks
        .iter()
        .map(|chunk| match chunk {
            SseChunk::Bytes(bytes) => {
                let text = str::from_utf8(bytes).or_value_error()?;
                Ok(json!({"bytes": text}))
            }
            SseChunk::Event(event) => Ok(json!({"event": event})),
            SseChunk::Other => Ok(Value::Null),
        })
        .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
    Ok(Value::Array(values))
}

fn text_answer(scenario: &TextScenario) -> HelperResult {
    let mut chunks: Vec<SseChunk> = scenario.responses_so_far.iter().map(chunk_of).collect();
    let outcome = rewrite_ended_stream_text(
        &mut chunks,
        &scenario.rewritten_text,
        &scenario.guardrail_name,
    );
    let error = match outcome {
        Ok(()) => Value::Null,
        Err(failure) => Value::String(failure.to_string()),
    };
    Ok(json!({"chunks": encode_chunks(&chunks)?, "error": error}))
}

fn tool_answer(scenario: &ToolScenario) -> HelperResult {
    let mut chunks: Vec<SseChunk> = scenario.responses_so_far.iter().map(chunk_of).collect();
    let pre = shapes(&scenario.pre_guardrail_tool_calls);
    let post = shapes(&scenario.post_guardrail_tool_calls);
    let name = scenario.guardrail_name.as_str();
    let outcome = rewrite_ended_tool_calls(&mut chunks, &pre, &post, name);
    let error = match outcome {
        Ok(()) => Value::Null,
        Err(failure) => Value::String(failure.to_string()),
    };
    Ok(json!({"chunks": encode_chunks(&chunks)?, "error": error}))
}

fn usage_answer(scenario: &UsageScenario) -> HelperResult {
    let chunks: Vec<SseChunk> = scenario.responses_so_far.iter().map(chunk_of).collect();
    let request_data: Option<BTreeMap<String, GuardrailValue>> = scenario
        .request_data
        .as_ref()
        .map(|fields| serde_json::from_value(Value::Object(fields.clone())))
        .transpose()
        .or_value_error()?;
    let response = build_streaming_usage_response(&chunks, request_data.as_ref());
    let projected = match response {
        Some(value) => {
            let text = serde_json::to_value(&value).or_value_error()?;
            Some(serde_json::from_value::<ProjectedResponse>(text).or_value_error()?)
        }
        None => None,
    };
    Ok(json!({"response": projected}))
}

impl Helper for Op {
    type Input = Function;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = match input {
            Function::TextRewrite { scenarios } => scenarios
                .iter()
                .map(text_answer)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            Function::ToolRewrite { scenarios } => scenarios
                .iter()
                .map(tool_answer)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            Function::UsageResponse { scenarios } => scenarios
                .iter()
                .map(usage_answer)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
        };
        Ok(Value::Array(answers))
    }
}
