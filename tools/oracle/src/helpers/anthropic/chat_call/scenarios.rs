use std::convert::Infallible;
use std::time::Duration;

use betterlite_anthropic::anthropic_http_stream::completion::WrappedStream;
use betterlite_anthropic::anthropic_http_stream::transport::StreamPost;
use betterlite_anthropic::chat_call::error::ChatCallError;
use betterlite_anthropic::chat_call::transport::{ClientKind, SuppliedChatClient};
use betterlite_anthropic::chat_call::{
    ChatCall, ChatClients, ChatDeps, ChatOutcome, EMBEDDING_REPLY, chat_call,
};
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::Input;
use super::fixture::runtime::{
    FixtureCounter, FixtureIds, Silent, logging_env, logging_state, redactor,
};
use super::fixture::{
    FixtureConfig, FixtureFailure, FixtureReply, FixtureTransport, Logger, LoggerCall, Recorded,
    Trip,
};
use super::support::{
    failure_post_call, header_map, logger_value, post_call_value, pre_call_value, reply_value,
    request_value, stream_value, transform_value,
};

const NOW: i64 = 1_767_225_600;
const TIMEOUT: Duration = Duration::from_secs(1);
const MAX_BYTES: usize = 0x0010_0000;

type CallResult =
    Result<ChatOutcome<FixtureFailure, FixtureCounter>, ChatCallError<FixtureFailure, Infallible>>;

enum RunOutcome {
    Reply(Box<ResponseEnvelope>),
    Stream(Box<WrappedStream<FixtureFailure, FixtureCounter>>),
    Failed(Option<u16>),
}

struct Run {
    outcome: RunOutcome,
    posts: Vec<StreamPost>,
    details: IndexMap<String, Json>,
    transforms: Vec<Recorded>,
    logger_calls: Vec<LoggerCall>,
}

const fn client_kind(acompletion: bool) -> ClientKind {
    if acompletion {
        ClientKind::Async
    } else {
        ClientKind::Sync
    }
}

fn reply_of(input: &Input) -> Result<FixtureReply, Box<HelperError>> {
    Ok(FixtureReply {
        status_code: input.upstream_status.unwrap_or(200),
        headers: header_map(&input.upstream_headers)?,
        body: input.upstream_body.clone().unwrap_or_default().into_bytes(),
        lines: input.upstream_lines.clone(),
    })
}

fn trip_of(input: &Input) -> Result<Trip, Box<HelperError>> {
    Ok(match (input.upstream_status, &input.failure_message) {
        (Some(status_code), _) => Trip::Status {
            status_code,
            headers: header_map(&input.upstream_headers)?,
            body: input.upstream_body.clone().unwrap_or_default().into_bytes(),
        },
        (None, Some(message)) => Trip::Unknown(message.clone()),
        (None, None) => Trip::Reply,
    })
}

fn call_of<'call>(
    input: &'call Input,
    settings: &'call AnthropicSettings,
    api_key: Option<&'call SecretString>,
    acompletion: bool,
) -> ChatCall<'call> {
    ChatCall {
        model: &input.model,
        custom_llm_provider: "anthropic",
        messages: input.messages.clone(),
        optional_params: input.optional_params.clone(),
        litellm_params: input.litellm_params.clone(),
        caller_headers: input.caller_headers.clone(),
        api_base: &input.api_base,
        api_key,
        settings,
        timeout: TIMEOUT,
        acompletion,
        effort_used: false,
        now: NOW,
        disable_add_prefix_to_prompt: false,
        model_response: ResponseEnvelope::new("chatcmpl-1".to_owned()),
    }
}

fn run_of(
    result: CallResult,
    transport: &FixtureTransport,
    state: &LoggingState,
    config: &FixtureConfig,
    logger: &Logger,
) -> Run {
    let outcome = match result {
        Ok(ChatOutcome::Reply(envelope)) => RunOutcome::Reply(envelope),
        Ok(ChatOutcome::Stream(wrapper)) => RunOutcome::Stream(wrapper),
        Err(error) => RunOutcome::Failed(error.status_code()),
    };
    Run {
        outcome,
        posts: transport.take_posts(),
        details: state.model_call_details.clone(),
        transforms: config.take_transforms(),
        logger_calls: logger.take(),
    }
}

fn execute(input: &Input, acompletion: bool, kind: ClientKind) -> Result<Run, Box<HelperError>> {
    let redactor = redactor()?;
    let silent = Silent;
    let env = logging_env(&redactor, &silent)?;
    let mut state = logging_state(&redactor)?;
    let transport = FixtureTransport::new(reply_of(input)?, trip_of(input)?);
    let config = FixtureConfig::new(input)?;
    let logger = Logger::default();
    let settings = AnthropicSettings::default();
    let api_key = input
        .api_key
        .as_ref()
        .map(|text| SecretString::from(text.clone()));
    let call = call_of(input, &settings, api_key.as_ref(), acompletion);
    let deps = ChatDeps {
        config: &config,
        clients: ChatClients {
            supplied: Some(SuppliedChatClient {
                kind,
                transport: &transport,
            }),
            pooled: &transport,
        },
        state: &mut state,
        env: &env,
        stream_logger: &logger,
        counter: FixtureCounter,
        ids: &FixtureIds,
        max_event_bytes: MAX_BYTES,
        max_error_body_bytes: MAX_BYTES,
    };
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let result = runtime.block_on(chat_call(call, deps));
    Ok(run_of(result, &transport, &state, &config, &logger))
}

pub(super) fn messages_post_request(input: &Input) -> HelperResult {
    Ok(json!({
        "async": request_bundle(input, true)?,
        "sync": request_bundle(input, false)?,
    }))
}

fn request_bundle(input: &Input, acompletion: bool) -> Result<Value, Box<HelperError>> {
    let run = execute(input, acompletion, client_kind(acompletion))?;
    let transform = run.transforms.first().map_or(Value::Null, |record| {
        transform_value(record, &input.request_header_names)
    });
    let request = run.posts.first().map_or(Value::Null, |post| {
        request_value(post, &input.request_header_names)
    });
    Ok(json!({
        "transform": transform,
        "pre_call": pre_call_value(&run.details),
        "request": request,
    }))
}

pub(super) fn buffered_success(input: &Input) -> HelperResult {
    Ok(json!({
        "async": reply_bundle(input, true)?,
        "sync": reply_bundle(input, false)?,
    }))
}

fn reply_bundle(input: &Input, acompletion: bool) -> Result<Value, Box<HelperError>> {
    let run = execute(input, acompletion, client_kind(acompletion))?;
    let RunOutcome::Reply(envelope) = &run.outcome else {
        return Err("the buffered call did not return a reply".to_owned()).or_value_error();
    };
    Ok(json!({
        "reply": reply_value(envelope),
        "post_call": post_call_value(&run.details),
    }))
}

pub(super) fn stream_request(input: &Input) -> HelperResult {
    Ok(json!({
        "async": stream_bundle(input, true)?,
        "sync": stream_bundle(input, false)?,
    }))
}

fn stream_bundle(input: &Input, acompletion: bool) -> Result<Value, Box<HelperError>> {
    let run = execute(input, acompletion, client_kind(acompletion))?;
    let RunOutcome::Stream(wrapper) = &run.outcome else {
        return Err("the stream call did not open a stream".to_owned()).or_value_error();
    };
    let request = run.posts.first().map_or(Value::Null, |post| {
        request_value(post, &input.request_header_names)
    });
    Ok(json!({
        "stream": stream_value(wrapper, &input.response_header_names),
        "pre_call": pre_call_value(&run.details),
        "request": request,
        "post_call": run.logger_calls.first().map_or(Value::Null, logger_value),
    }))
}

fn failure_status(outcome: &RunOutcome) -> Option<u16> {
    match outcome {
        RunOutcome::Failed(status) => *status,
        RunOutcome::Reply(_) | RunOutcome::Stream(_) => None,
    }
}

pub(super) fn pre_stream_error(input: &Input) -> HelperResult {
    let run = execute(input, input.acompletion, client_kind(input.acompletion))?;
    Ok(json!({
        "status": failure_status(&run.outcome),
        "posted": run.posts.len(),
        "transformed": !run.transforms.is_empty(),
    }))
}

pub(super) fn native_error_body(input: &Input) -> HelperResult {
    let run = execute(input, input.acompletion, client_kind(input.acompletion))?;
    Ok(json!({
        "status": failure_status(&run.outcome),
        "posted": run.posts.len(),
        "post_call": post_call_value(&run.details),
    }))
}

pub(super) fn unsupported_embedding() -> HelperResult {
    Ok(json!({"reply": EMBEDDING_REPLY, "posted": 0}))
}

pub(super) fn chat_failure_projection(input: &Input) -> HelperResult {
    let asynchronous = execute(input, true, ClientKind::Async)?;
    let synchronous = execute(input, false, ClientKind::Sync)?;
    Ok(json!({
        "async": failure_bundle(&asynchronous),
        "sync": failure_bundle(&synchronous),
    }))
}

fn failure_bundle(run: &Run) -> Value {
    json!({
        "status": failure_status(&run.outcome),
        "posted": run.posts.len(),
        "post_call": failure_post_call(&run.details),
    })
}
