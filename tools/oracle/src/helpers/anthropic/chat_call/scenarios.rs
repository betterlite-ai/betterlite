use std::convert::Infallible;
use std::mem::take;
use std::sync::PoisonError;
use std::time::Duration;

use betterlite_anthropic::anthropic_http_stream::completion::WrappedStream;
use betterlite_anthropic::anthropic_http_stream::transport::StreamPost;
use betterlite_anthropic::chat_call::error::ChatCallError;
use betterlite_anthropic::chat_call::{
    CallStyle, ChatCall, ChatClients, ChatDeps, ChatOutcome, chat_call,
};
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::client_kind::SuppliedClient;
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
use super::failures::FailureShape;
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
    Embedding,
    Failed,
}
pub(super) struct Run {
    outcome: RunOutcome,
    pub(super) failure: Option<FailureShape>,
    posts: Vec<StreamPost>,
    details: IndexMap<String, Json>,
    pre_call: IndexMap<String, Json>,
    logger_calls: Vec<LoggerCall>,
}
impl Run {
    fn reply(&self) -> Result<&ResponseEnvelope, Box<HelperError>> {
        match &self.outcome {
            RunOutcome::Reply(envelope) => Ok(envelope),
            RunOutcome::Stream(_) | RunOutcome::Embedding | RunOutcome::Failed => {
                Err("the buffered call did not return a reply").or_value_error()
            }
        }
    }

    fn stream(&self) -> Result<&FixtureStream, Box<HelperError>> {
        match &self.outcome {
            RunOutcome::Stream(wrapper) => Ok(wrapper),
            RunOutcome::Reply(_) | RunOutcome::Embedding | RunOutcome::Failed => {
                Err("the stream call did not open a stream").or_value_error()
            }
        }
    }
}
pub(super) const fn client_kind(acompletion: bool) -> ClientKind {
    if acompletion {
        ClientKind::Asynchronous
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
    // httpx only raises on a non-2xx status, so a 2xx is a reply even when the fixture names a message
    let trip = match (input.upstream_status, &input.failure_message) {
        (None, Some(message)) => Trip::Unknown(message.clone()),
        (Some(status_code), _) if !(200..300).contains(&status_code) => Trip::Status {
            status_code,
            headers,
            body,
        },
        _ => Trip::Reply,
    };
    Ok((reply, trip))
}
fn call_of<'call>(
    input: &'call Input,
    settings: &'call AnthropicSettings,
    api_key: Option<&'call SecretString>,
    style: CallStyle,
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
        style,
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
    pre_call_record: Option<IndexMap<String, Json>>,
) -> Run {
    let (outcome, failure) = match result {
        Ok(ChatOutcome::Reply(envelope)) => (RunOutcome::Reply(envelope), None),
        Ok(ChatOutcome::Stream(wrapper)) => (RunOutcome::Stream(wrapper), None),
        Ok(ChatOutcome::Embedding) => (RunOutcome::Embedding, None),
        Err(error) => {
            let shape = FailureShape::of(&error);
            (RunOutcome::Failed, Some(shape))
        }
    };
    let details = state.model_call_details.clone();
    // the response mapper's post_call replaces the record, so a buffered reply keeps the fixture's copy
    let pre_call = pre_call_record.unwrap_or_else(|| details.clone());
    Run {
        outcome,
        failure,
        posts: take(
            &mut *transport
                .posts
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        ),
        details,
        pre_call,
        logger_calls: take(&mut *logger.calls.lock().unwrap_or_else(PoisonError::into_inner)),
    }
}
pub(super) fn execute(
    input: &Input,
    style: CallStyle,
    kind: ClientKind,
) -> Result<Run, Box<HelperError>> {
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
    let call = call_of(input, &settings, api_key.as_ref(), style);
    let deps = ChatDeps {
        config: &config,
        clients: ChatClients {
            supplied: Some(SuppliedClient {
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
    let pre_call = config.pre_call_record();
    Ok(run_of(result, &transport, &state, &logger, pre_call))
}

fn both(input: &Input) -> Result<(Run, Run), Box<HelperError>> {
    let asynchronous = execute(input, CallStyle::Async, ClientKind::Asynchronous)?;
    let synchronous = execute(input, CallStyle::Sync, ClientKind::Sync)?;
    Ok((asynchronous, synchronous))
}

pub(super) fn unsupported_embedding(input: &Input) -> HelperResult {
    let run = execute(input, CallStyle::Embedding, ClientKind::Sync)?;
    let RunOutcome::Embedding = run.outcome else {
        return Err("the embedding call did not return its no-op result").or_value_error();
    };
    Ok(json!({"reply": null}))
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
            "pre_call": pre_call_value(&asynchronous.pre_call),
            "request": request_of(input, &asynchronous),
        },
        "sync": {
            "pre_call": pre_call_value(&synchronous.pre_call),
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
        "pre_call": pre_call_value(&run.pre_call),
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
