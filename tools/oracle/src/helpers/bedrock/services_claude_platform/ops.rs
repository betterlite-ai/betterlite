use std::convert::Infallible;

use betterlite_anthropic::stream_decode::error::ReceiveError;
use betterlite_anthropic::stream_decode::{DecodedChunk, TransportChunk};
use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_bedrock::claude_platform::WORKSPACE_HEADER;
use betterlite_bedrock::claude_platform::chat::ChatHeadersRequest;
use betterlite_bedrock::claude_platform::convert;
use betterlite_bedrock::claude_platform::error::{ChatValidationError, NativeHeaderError};
use betterlite_bedrock::claude_platform::native::{
    NativeHeadersRequest, validate_environment as validate_native,
};
use betterlite_bedrock::claude_platform::stream::{StreamFlags, get_model_response_iterator};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;
use serde_json::{Value, json};
use uuid::Uuid;

use super::fixture::Fixture;
use super::input::{BodyInput, HeaderInput, Input, WorkspaceCase};
use super::project::{body_view, failure_view, project_headers};
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

#[derive(Clone, Copy)]
enum Route {
    Chat,
    Native,
}

// The four request-preparing methods share one fixture, so their seams stay in one function.
#[derive(Clone, Copy)]
enum Request<'request> {
    Headers(&'request HeaderInput, Route),
    Body(&'request BodyInput, Route),
}

fn workspaces(model: &str, cases: &[WorkspaceCase]) -> HelperResult {
    let fixture = Fixture::empty()?;
    let mut outcomes = Vec::with_capacity(cases.len());
    for case in cases {
        let platform =
            platform_inputs(&case.deployment, &case.litellm_settings, &case.environment)?;
        let no_headers = IndexMap::new();
        let chat = match fixture.chat_headers(&ChatHeadersRequest {
            model,
            messages: &[],
            optional_params: &case.optional_params,
            caller_headers: &no_headers,
            api_key: None,
            workspace: platform.sources(),
        }) {
            Ok(headers) => json!({"workspace": headers.values.get(WORKSPACE_HEADER)}),
            Err(error) => {
                let failed = match &error {
                    ChatValidationError::Workspace(workspace) => workspace.model(),
                    ChatValidationError::Features(_) | ChatValidationError::Json(_) => None,
                };
                failure_view(error.status_code(), failed.unwrap_or_default())
            }
        };
        let native_params: IndexMap<String, PromptJson> =
            convert::mapped(&case.optional_params).map_err(invalid)?;
        let no_native_messages: Vec<PromptJson> = Vec::new();
        let native = match validate_native(&NativeHeadersRequest {
            model,
            caller_headers: &no_headers,
            messages: &no_native_messages,
            optional_params: &native_params,
            litellm_params: &IndexMap::new(),
            api_key: None,
            api_base: None,
            workspace: platform.sources(),
        }) {
            Ok(headers) => json!({"workspace": headers.headers.get(WORKSPACE_HEADER)}),
            Err(error) => {
                let failed = match &error {
                    NativeHeaderError::Workspace(workspace) => workspace.model(),
                    NativeHeaderError::Beta(_) | NativeHeaderError::Json(_) => None,
                };
                failure_view(error.status_code(), failed.unwrap_or_default())
            }
        };
        outcomes.push(json!({"chat": chat, "native": native}));
    }
    Ok(json!({"outcomes": outcomes}))
}

fn inspect(request: Request<'_>) -> HelperResult {
    let fixture = Fixture::empty()?;
    match request {
        Request::Headers(input, Route::Chat) => {
            let platform = platform_inputs(
                &input.deployment,
                &input.litellm_settings,
                &input.environment,
            )?;
            match fixture.chat_headers(&ChatHeadersRequest {
                model: &input.model,
                messages: &input.messages,
                optional_params: &input.optional_params,
                caller_headers: &input.headers,
                api_key: input.api_key.as_ref(),
                workspace: platform.sources(),
            }) {
                Ok(headers) => Ok(json!({"headers": project_headers(&headers.values)})),
                Err(error) => {
                    let failed = match &error {
                        ChatValidationError::Workspace(workspace) => workspace.model(),
                        ChatValidationError::Features(_) | ChatValidationError::Json(_) => None,
                    };
                    Ok(failure_view(
                        error.status_code(),
                        failed.unwrap_or_default(),
                    ))
                }
            }
        }
        Request::Headers(input, Route::Native) => {
            let platform = platform_inputs(
                &input.deployment,
                &input.litellm_settings,
                &input.environment,
            )?;
            let messages = convert::prompt_messages(&input.messages).map_err(invalid)?;
            let optional_params: IndexMap<String, PromptJson> =
                convert::mapped(&input.optional_params).map_err(invalid)?;
            match validate_native(&NativeHeadersRequest {
                model: &input.model,
                caller_headers: &input.headers,
                messages: &messages,
                optional_params: &optional_params,
                litellm_params: &input.litellm_params,
                api_key: input.api_key.as_ref(),
                api_base: input.api_base.as_deref(),
                workspace: platform.sources(),
            }) {
                Ok(headers) => Ok(json!({
                    "headers": project_headers(&headers.headers),
                    "api_base": headers.api_base,
                })),
                Err(error) => {
                    let failed = match &error {
                        NativeHeaderError::Workspace(workspace) => workspace.model(),
                        NativeHeaderError::Beta(_) | NativeHeaderError::Json(_) => None,
                    };
                    Ok(failure_view(
                        error.status_code(),
                        failed.unwrap_or_default(),
                    ))
                }
            }
        }
        Request::Body(input, Route::Chat) => {
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
        Request::Body(input, Route::Native) => {
            let messages = convert::prompt_messages(&input.messages).map_err(invalid)?;
            let optional_params: IndexMap<String, PromptJson> =
                convert::mapped(&input.optional_params).map_err(invalid)?;
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
    }
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
    let chunks = get_model_response_iterator(
        source,
        StreamFlags {
            sync_stream: false,
            json_mode,
        },
        Counting,
        &ids,
        MAX_EVENT_BYTES,
    );
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
        Input::ChatPlatformHeaders(input) => inspect(Request::Headers(input, Route::Chat)),
        Input::NativePlatformHeaders(input) => inspect(Request::Headers(input, Route::Native)),
        Input::ChatPlatformBody(input) => inspect(Request::Body(input, Route::Chat)),
        Input::NativePlatformBody(input) => inspect(Request::Body(input, Route::Native)),
        Input::NativeAnthropicStream {
            id,
            json_mode,
            lines,
        } => stream(id, *json_mode, lines),
    }
}
