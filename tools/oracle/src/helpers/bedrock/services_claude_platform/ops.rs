use std::convert::Infallible;

use betterlite_anthropic::stream_decode::error::ReceiveError;
use betterlite_anthropic::stream_decode::{DecodedChunk, TransportChunk};
use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_bedrock::claude_platform::convert;
use betterlite_bedrock::claude_platform::stream::get_model_response_iterator;
use betterlite_oracle_helper_api::helper::HelperResult;
use indexmap::IndexMap;
use serde_json::{Value, json};
use uuid::Uuid;

use super::fixture::Fixture;
use super::input::{BodyInput, HeaderInput, Input, WorkspaceCase};
use super::project::{
    body_view, chat_failure, native_header_failure, project_headers, workspace_view,
};
use super::stubs::{invalid, platform_inputs};

const MAX_EVENT_BYTES: usize = 0x0010_0000;

struct FixedIds {
    value: Uuid,
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        self.value
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the case counts no reasoning tokens")]
struct CounterError;

struct Counting;

impl ReasoningTokenCounter for Counting {
    type Failure = CounterError;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, CounterError> {
        Ok(0)
    }
}

fn workspaces(model: &str, cases: &[WorkspaceCase]) -> HelperResult {
    let fixture = Fixture::empty()?;
    let mut outcomes = Vec::with_capacity(cases.len());
    for case in cases {
        let platform =
            platform_inputs(&case.deployment, &case.litellm_settings, &case.environment)?;
        let no_headers = IndexMap::new();
        let chat = match fixture.chat_headers(
            model,
            &[],
            &case.optional_params,
            &no_headers,
            None,
            &platform,
        ) {
            Ok(headers) => workspace_view(&headers.values),
            Err(error) => chat_failure(&error),
        };
        let native_params = convert::prompt_params(&case.optional_params).map_err(invalid)?;
        let no_native_messages: Vec<betterlite_prompts::arguments::json::Json> = Vec::new();
        let native = match fixture.native_headers(
            model,
            &no_headers,
            &no_native_messages,
            &native_params,
            &IndexMap::new(),
            None,
            None,
            &platform,
        ) {
            Ok(headers) => workspace_view(&headers.headers),
            Err(error) => native_header_failure(&error),
        };
        outcomes.push(json!({"chat": chat, "native": native}));
    }
    Ok(json!({"outcomes": outcomes}))
}

fn chat_headers(input: &HeaderInput) -> HelperResult {
    let fixture = Fixture::empty()?;
    let platform = platform_inputs(
        &input.deployment,
        &input.litellm_settings,
        &input.environment,
    )?;
    match fixture.chat_headers(
        &input.model,
        &input.messages,
        &input.optional_params,
        &input.headers,
        input.api_key.as_ref(),
        &platform,
    ) {
        Ok(headers) => Ok(json!({"headers": project_headers(&headers.values)})),
        Err(error) => Ok(chat_failure(&error)),
    }
}

fn native_headers(input: &HeaderInput) -> HelperResult {
    let fixture = Fixture::empty()?;
    let platform = platform_inputs(
        &input.deployment,
        &input.litellm_settings,
        &input.environment,
    )?;
    let messages = convert::prompt_messages(&input.messages).map_err(invalid)?;
    let optional_params = convert::prompt_params(&input.optional_params).map_err(invalid)?;
    match fixture.native_headers(
        &input.model,
        &input.headers,
        &messages,
        &optional_params,
        &input.litellm_params,
        input.api_key.as_ref(),
        input.api_base.as_deref(),
        &platform,
    ) {
        Ok(headers) => Ok(json!({
            "headers": project_headers(&headers.headers),
            "api_base": headers.api_base,
        })),
        Err(error) => Ok(native_header_failure(&error)),
    }
}

fn chat_body(input: &BodyInput) -> HelperResult {
    let fixture = Fixture::empty()?;
    let prepared = fixture
        .chat_body(
            &input.model,
            input.messages.clone(),
            input.optional_params.clone(),
            input.litellm_params.clone(),
            input.headers.clone(),
        )
        .map_err(invalid)?;
    let body = serde_json::to_value(&prepared.prepared.body).map_err(invalid)?;
    Ok(body_view(
        &body,
        &prepared.warnings,
        &prepared.prepared.headers.values,
    ))
}

fn native_body(input: &BodyInput) -> HelperResult {
    let fixture = Fixture::empty()?;
    let messages = convert::prompt_messages(&input.messages).map_err(invalid)?;
    let optional_params = convert::prompt_params(&input.optional_params).map_err(invalid)?;
    let prepared = fixture
        .native_body(
            &input.model,
            messages,
            optional_params,
            input.litellm_params.clone(),
        )
        .map_err(invalid)?;
    let body = serde_json::to_value(&prepared.body).map_err(invalid)?;
    Ok(body_view(&body, &prepared.warnings, &IndexMap::new()))
}

fn chunk_view(event: &StreamEventChunk) -> Value {
    let choice = event.choices.first();
    let content = choice
        .map(|selected| selected.delta.content.clone())
        .unwrap_or_default();
    let finish_reason = choice.and_then(|selected| selected.finish_reason.clone());
    json!({
        "id": &event.id,
        "model": &event.model,
        "content": content,
        "finish_reason": finish_reason,
    })
}

fn stream(id: &str, json_mode: Option<bool>, lines: &[String]) -> HelperResult {
    let ids = FixedIds {
        value: Uuid::parse_str(id).map_err(invalid)?,
    };
    let source = lines.iter().map(|line| {
        Ok::<TransportChunk, ReceiveError<Infallible>>(TransportChunk::Text(line.clone()))
    });
    let chunks =
        get_model_response_iterator(source, false, json_mode, Counting, &ids, MAX_EVENT_BYTES);
    let mut recorded = Vec::new();
    for outcome in chunks {
        match outcome {
            Ok(DecodedChunk::Event(event)) => recorded.push(chunk_view(&event)),
            Ok(DecodedChunk::Generic(_)) => {
                return Err(invalid("the case produced a generic chunk"));
            }
            Err(error) => return Err(invalid(error)),
        }
    }
    Ok(json!({"chunks": recorded}))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::WorkspaceIdRequired { model, cases } => workspaces(model, cases),
        Input::ChatPlatformHeaders(input) => chat_headers(input),
        Input::NativePlatformHeaders(input) => native_headers(input),
        Input::ChatPlatformBody(input) => chat_body(input),
        Input::NativePlatformBody(input) => native_body(input),
        Input::NativeAnthropicStream {
            id,
            json_mode,
            lines,
        } => stream(id, *json_mode, lines),
    }
}
