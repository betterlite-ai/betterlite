use std::convert::Infallible;
use std::sync::PoisonError;
use std::time::Duration;

use betterlite_anthropic::anthropic_http_stream::completion::WrappedStream;
use betterlite_anthropic::anthropic_http_stream::transport::StreamPost;
use betterlite_anthropic::chat_call::error::ChatCallError;
use betterlite_anthropic::chat_call::transport::{ClientKind, SuppliedChatClient};
use betterlite_anthropic::chat_call::{ChatCall, ChatClients, ChatDeps, ChatOutcome, chat_call};
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::headers::HeadersError;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::Input;
use super::fixture::runtime::{FixtureCounter, FixtureIds, Silent, logging_env, logging_state};
use super::fixture::{
    FixtureConfig, FixtureFailure, FixtureReply, FixtureTransport, Logger, LoggerCall, Trip,
};
use super::support::{
    header_map, logger_value, post_call_value, pre_call_value, reply_value, request_value,
    stream_value,
};

const NOW: i64 = 1_767_225_600;
const TIMEOUT: Duration = Duration::from_secs(1);
const MAX_BYTES: usize = 0x0010_0000;

type CallResult =
    Result<ChatOutcome<FixtureFailure, FixtureCounter>, ChatCallError<FixtureFailure, Infallible>>;
type FixtureStream = WrappedStream<FixtureFailure, FixtureCounter>;

enum RunOutcome {
    Reply(Box<ResponseEnvelope>),
    Stream(Box<FixtureStream>),
    Failed,
}

struct Run {
    outcome: RunOutcome,
    failure: Option<FailureShape>,
    posts: Vec<StreamPost>,
    details: IndexMap<String, Json>,
    logger_calls: Vec<LoggerCall>,
}

// the exception LiteLLM raises for the same call, named the way Python names it
struct FailureShape {
    class: &'static str,
    status: Option<u16>,
    message: String,
}

impl FailureShape {
    fn of(error: &ChatCallError<FixtureFailure, Infallible>) -> Self {
        let class = match error {
            ChatCallError::Anthropic { .. } => "AnthropicError",
            ChatCallError::Headers(HeadersError::Authentication { .. }) => "AuthenticationError",
            ChatCallError::Headers(_) => "HeadersError",
            ChatCallError::Prepare { .. } | ChatCallError::Filter { .. } => "ValueError",
            ChatCallError::PreCall(_) => "PreCallError",
            ChatCallError::Response(_) | ChatCallError::Parameter(_) => "ValueError",
            ChatCallError::Open(_) => "AnthropicError",
            ChatCallError::Endpoint { .. }
            | ChatCallError::HeaderName { .. }
            | ChatCallError::HeaderValue { .. } => "ValueError",
        };
        Self {
            class,
            status: error.status_code(),
            message: error.to_string(),
        }
    }
}

impl Run {
    fn reply(&self) -> Result<&ResponseEnvelope, Box<HelperError>> {
        match &self.outcome {
            RunOutcome::Reply(envelope) => Ok(envelope),
            _ => "the buffered call did not return a reply"
                .to_owned()
                .or_value_error(),
        }
    }

    fn stream(&self) -> Result<&FixtureStream, Box<HelperError>> {
        match &self.outcome {
            RunOutcome::Stream(wrapper) => Ok(wrapper),
            _ => "the stream call did not open a stream"
                .to_owned()
                .or_value_error(),
        }
    }
}

const fn client_kind(acompletion: bool) -> ClientKind {
    if acompletion {
        ClientKind::Async
    } else {
        ClientKind::Sync
    }
}

// the same fixture answers the reply and the trip, so the two stay one endpoint
fn endpoint_of(input: &Input) -> Result<(FixtureReply, Trip), Box<HelperError>> {
    let headers = header_map(&input.upstream_headers)?;
    let body = input.upstream_body.clone().unwrap_or_default().into_bytes();
    let reply = FixtureReply {
        status_code: input.upstream_status.unwrap_or(200),
        headers: headers.clone(),
        body: body.clone(),
        lines: input.upstream_lines.clone(),
    };
    let trip = match (input.upstream_status, &input.failure_message) {
        (Some(status_code), _) => Trip::Status {
            status_code,
            headers,
            body,
        },
        (None, Some(message)) => Trip::Unknown(message.clone()),
        (None, None) => Trip::Reply,
    };
    Ok((reply, trip))
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
    logger: &Logger,
) -> Run {
    let (outcome, failure) = match result {
        Ok(ChatOutcome::Reply(envelope)) => (RunOutcome::Reply(envelope), None),
        Ok(ChatOutcome::Stream(wrapper)) => (RunOutcome::Stream(wrapper), None),
        Err(error) => {
            let shape = FailureShape::of(&error);
            (RunOutcome::Failed, Some(shape))
        }
    };
    Run {
        outcome,
        failure,
        posts: std::mem::take(
            &mut *transport
                .posts
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        ),
        details: state.model_call_details.clone(),
        logger_calls: std::mem::take(
            &mut *logger.calls.lock().unwrap_or_else(PoisonError::into_inner),
        ),
    }
}

fn execute(input: &Input, acompletion: bool, kind: ClientKind) -> Result<Run, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let silent = Silent;
    let env = logging_env(&redactor, &silent)?;
    let mut state = logging_state(&redactor)?;
    let (reply, trip) = endpoint_of(input)?;
    let transport = FixtureTransport::new(reply, trip);
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
    Ok(run_of(result, &transport, &state, &logger))
}

fn both(input: &Input) -> Result<(Run, Run), Box<HelperError>> {
    let asynchronous = execute(input, true, ClientKind::Async)?;
    let synchronous = execute(input, false, ClientKind::Sync)?;
    Ok((asynchronous, synchronous))
}

fn request_of(input: &Input, run: &Run) -> Value {
    let names = &input.request_header_names;
    run.posts
        .first()
        .map_or(Value::Null, |post| request_value(post, names))
}

pub(super) fn messages_post_request(input: &Input) -> HelperResult {
    let (asynchronous, synchronous) = both(input)?;
    Ok(json!({
        "async": {
            "pre_call": pre_call_value(&asynchronous.details),
            "request": request_of(input, &asynchronous),
        },
        "sync": {
            "pre_call": pre_call_value(&synchronous.details),
            "request": request_of(input, &synchronous),
        },
    }))
}

fn reply_answer(run: &Run) -> Result<Value, Box<HelperError>> {
    let envelope = run.reply()?;
    Ok(json!({
        "reply": reply_value(envelope),
        "post_call": post_call_value(&run.details),
    }))
}

pub(super) fn buffered_success(input: &Input) -> HelperResult {
    let (asynchronous, synchronous) = both(input)?;
    Ok(json!({
        "async": reply_answer(&asynchronous)?,
        "sync": reply_answer(&synchronous)?,
    }))
}

fn stream_answer(input: &Input, run: &Run) -> Result<Value, Box<HelperError>> {
    let wrapper = run.stream()?;
    let logged = run.logger_calls.first().map_or(Value::Null, logger_value);
    Ok(json!({
        "stream": stream_value(wrapper),
        "pre_call": pre_call_value(&run.details),
        "request": request_of(input, run),
        "post_call": logged,
    }))
}

pub(super) fn stream_request(input: &Input) -> HelperResult {
    let (asynchronous, synchronous) = both(input)?;
    let async_answer = stream_answer(input, &asynchronous)?;
    let sync_answer = stream_answer(input, &synchronous)?;
    Ok(json!({"async": async_answer, "sync": sync_answer}))
}

// the case compares the exception LiteLLM raises, so the answer is that error, not a value
fn failure_answer(run: &Run) -> HelperResult {
    match &run.failure {
        Some(failure) => Err(Box::new(HelperError {
            class: failure.class.to_owned(),
            status: failure.status,
            error_type: None,
            param: None,
            code: None,
            message: failure.message.clone(),
        })),
        None => "the call was expected to fail and did not"
            .to_owned()
            .or_value_error(),
    }
}

pub(super) fn pre_stream_error(input: &Input) -> HelperResult {
    let acompletion = input.acompletion;
    let run = execute(input, acompletion, client_kind(acompletion))?;
    failure_answer(&run)
}

pub(super) fn native_error_body(input: &Input) -> HelperResult {
    let acompletion = input.acompletion;
    let run = execute(input, acompletion, client_kind(acompletion))?;
    failure_answer(&run)
}

pub(super) fn chat_failure_projection(input: &Input) -> HelperResult {
    let asynchronous = execute(input, true, ClientKind::Async)?;
    if asynchronous.failure.is_none() {
        return "the call was expected to fail and did not"
            .to_owned()
            .or_value_error();
    }
    failure_answer(&asynchronous)
}
