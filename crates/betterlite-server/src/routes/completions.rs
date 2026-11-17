use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_context::call_context::{CallContext as DispatchContext, CallInputs};
use betterlite_core::dispatch::error::DispatchError;
use betterlite_core::dispatch::log::TracingCallLog;
use betterlite_core::dispatch::{Dispatch, NoRules};
use betterlite_core::operation::text_completion::{
    ChatCallAnswer, ChatCallMode, TextCompletionCall, TextCompletionCallInput,
    TextCompletionFailure, TextCompletionRequest, TextCompletionResult,
};
use betterlite_core::operation::text_stream::TextStreamAnswer;
use betterlite_core::operation::{CallResult, Operation};
use betterlite_http::proxy::StreamResponse;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatError;
use betterlite_openai::completion_chat_bridge::answer::{
    text_answer_body, text_completion_response,
};
use betterlite_openai::completion_chat_bridge::{CompletionEndpoint, chat_request};
use betterlite_openai::stream::SseParser;
use betterlite_openai::text_completion_execution::log::TracingCompletionLog;
use betterlite_openai::text_completion_execution::{
    CompletionAnswer, CompletionCallAnswer, CompletionExecutor, CompletionStreamAnswer,
};
use betterlite_openai::text_completion_request::{CompletionRequest, CompletionRequestError};
use betterlite_openai::text_completion_stream::{decode_chat_events, shaped_chunk};
use betterlite_provider::operation::ChatCompletion;
use futures_util::StreamExt as _;
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::routing::simple_shuffle::pick_deployment;
use crate::server::chat_setup::provider_call::{CallError, answer_chat};
use crate::state::ServerState;

use super::chat::MAX_CALL_ID_LENGTH;
use super::chat::headers::success_headers;

use self::error::{FailureAnswer, failure_answer, render};

mod error;
pub(crate) mod streaming;
mod tests;

pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    let executor = Arc::clone(&state.executor);
    let mut router = Router::new();
    router = router.route("/completions", post(completions));
    router = router.route("/v1/completions", post(completions));
    router = router.route("/engines/{model}/completions", post(completions));
    router = router.route("/openai/deployments/{model}/completions", post(completions));
    router.with_state(CompletionState {
        state: Arc::clone(state),
        execution: Arc::new(CompletionExecutor {
            client: executor.client.clone(),
            catalog: OnceCell::new(),
        }),
        dispatch: Arc::new(Dispatch::new(
            executor.deployments.clone(),
            Arc::new(NoRules),
            Arc::new(TracingCallLog),
        )),
        include_call_id: state
            .config
            .sections
            .error_body_call_id
            .include_call_id_in_error_body,
    })
}

#[derive(Clone)]
struct CompletionState {
    state: Arc<ServerState>,
    execution: Arc<CompletionExecutor>,
    dispatch: Arc<Dispatch>,
    include_call_id: bool,
}

// one per request: LiteLLM's completion route answers a failure with the call id header alone, never with the provider's
struct CompletionCalls {
    state: Arc<ServerState>,
    execution: Arc<CompletionExecutor>,
}

#[async_trait]
impl TextCompletionCall for CompletionCalls {
    async fn complete(
        &self,
        call: TextCompletionCallInput<'_>,
    ) -> Result<TextCompletionResult, TextCompletionFailure> {
        text_answer(self, call).await
    }

    async fn endpoint(&self, deployment: &ChatDeployment) -> CompletionEndpoint {
        self.execution.endpoint(deployment).await
    }

    async fn stream(
        &self,
        call: TextCompletionCallInput<'_>,
    ) -> Result<TextStreamAnswer, TextCompletionFailure> {
        text_stream(self, call).await
    }

    // a chat-catalog deployment answers or streams on the chat endpoint and converts back
    async fn chat(
        &self,
        call: TextCompletionCallInput<'_>,
        mode: ChatCallMode,
    ) -> Result<ChatCallAnswer, TextCompletionFailure> {
        let chat = chat_request(call.request).map_err(bridge_failure)?;
        match mode {
            ChatCallMode::Buffered => {
                let answered = answer_chat(
                    &self.state.executor,
                    &chat,
                    &call.deployment,
                    ControlOptions::default(),
                )
                .await;
                let (completion, upstream_headers, upstream_time) = match answered {
                    Ok(answer) => answer,
                    Err(error) => return Err(chat_call_failure(error, self)),
                };
                let completion = converted(completion, upstream_headers, upstream_time)?;
                Ok(ChatCallAnswer::Buffered(text_result(completion)))
            }
            ChatCallMode::Streamed => {
                let started = Instant::now();
                let upstream = streaming::provider_stream(
                    &self.state.executor,
                    &call.deployment,
                    &chat,
                    ControlOptions::default(),
                )
                .await
                .map_err(|error| chat_call_failure(error, self))?;
                let upstream_time = started.elapsed();
                let include_usage = call
                    .request
                    .stream_options
                    .as_ref()
                    .is_some_and(|options| options.include_usage);
                Ok(ChatCallAnswer::Streamed(chat_stream_answer(
                    upstream,
                    call.request.model.as_deref(),
                    include_usage,
                    upstream_time,
                )))
            }
        }
    }
}

async fn text_answer(
    calls: &CompletionCalls,
    call: TextCompletionCallInput<'_>,
) -> Result<TextCompletionResult, TextCompletionFailure> {
    let CompletionCallAnswer::Buffered(answer) = text_send(calls, &call, false).await? else {
        return Err(internal_failure(
            "the provider answered a different operation",
        ));
    };
    Ok(text_result(answer))
}

// the chat answer as the text completion response the route writes
fn converted(
    completion: ChatCompletion,
    upstream_headers: HeaderMap,
    upstream_time: Duration,
) -> Result<CompletionAnswer, TextCompletionFailure> {
    let ChatCompletion::OpenAi(response) = completion else {
        return Err(internal_failure(
            "the provider answered a different operation",
        ));
    };
    Ok(CompletionAnswer {
        response: Box::new(text_completion_response(*response)),
        upstream_headers,
        upstream_time,
    })
}

async fn text_stream(
    calls: &CompletionCalls,
    call: TextCompletionCallInput<'_>,
) -> Result<TextStreamAnswer, TextCompletionFailure> {
    streamed_answer(text_send(calls, &call, true).await)
        .map(|answer| stream_answer(answer, call.request.model.as_deref()))
}

// The text endpoint reached for this deployment, with the stream flag the dropped body loses
async fn text_send(
    calls: &CompletionCalls,
    call: &TextCompletionCallInput<'_>,
    streamed: bool,
) -> Result<CompletionCallAnswer, TextCompletionFailure> {
    let mut logger = TracingCompletionLog;
    let built = calls
        .execution
        .text_transport(&call.deployment, call.request, streamed)
        .await
        .map_err(bridge_failure)?;
    calls
        .execution
        .send(
            &call.deployment,
            built,
            streamed,
            &calls.state.executor.redactor,
            &mut logger,
        )
        .await
        .map_err(bridge_failure)
}

fn streamed_answer(
    answered: Result<CompletionCallAnswer, TextCompletionFailure>,
) -> Result<CompletionStreamAnswer, TextCompletionFailure> {
    match answered? {
        CompletionCallAnswer::Streamed(answer) => Ok(answer),
        CompletionCallAnswer::Buffered(_) => Err(internal_failure(
            "the provider answered a different operation",
        )),
    }
}

// the provider's buffered answer as the dispatch pipeline's text result
fn text_result(answer: CompletionAnswer) -> TextCompletionResult {
    TextCompletionResult {
        response: answer.response,
        upstream_headers: answer.upstream_headers,
        upstream_time: answer.upstream_time,
    }
}

// The text endpoint's chunks as the dispatch pipeline's stream answer, model restamped.
fn stream_answer(answer: CompletionStreamAnswer, model: Option<&str>) -> TextStreamAnswer {
    let model = model.unwrap_or_default().to_owned();
    let body = answer
        .stream
        .map(move |item| match item {
            Ok(chunk) => shaped_chunk(chunk, &model)
                .map_err(stream_failure)
                .and_then(|value| {
                    serde_json::from_value::<betterlite_models::decisions::Json>(value)
                        .map_err(stream_failure)
                }),
            Err(error) => Err(TextCompletionFailure {
                status: 500,
                error_type: None,
                param: None,
                message: error.to_string(),
            }),
        })
        .boxed();
    TextStreamAnswer {
        body,
        upstream_headers: answer.upstream_headers,
        upstream_time: answer.upstream_time,
    }
}

// The chat endpoint's bytes as the dispatch pipeline's stream answer the wrapper converts.
fn chat_stream_answer(
    upstream: StreamResponse,
    model: Option<&str>,
    include_usage: bool,
    upstream_time: Duration,
) -> TextStreamAnswer {
    let upstream_headers = upstream.response.headers().clone();
    let state = ChatChunkState {
        bytes: upstream.body_stream(),
        parser: SseParser::default(),
        model: model.unwrap_or_default().to_owned(),
        include_usage,
        pending: VecDeque::new(),
        finished: false,
    };
    TextStreamAnswer {
        body: futures_util::stream::unfold(state, next_chat_chunk).boxed(),
        upstream_headers,
        upstream_time,
    }
}

// The chat endpoint's SSE bytes read as the chat chunks the text wrapper converts, model restamped.
struct ChatChunkState {
    bytes: futures_util::stream::BoxStream<
        'static,
        Result<Vec<u8>, betterlite_http::error::HttpError>,
    >,
    parser: SseParser,
    model: String,
    include_usage: bool,
    pending: VecDeque<betterlite_models::decisions::Json>,
    finished: bool,
}

async fn next_chat_chunk(
    mut state: ChatChunkState,
) -> Option<(
    Result<betterlite_models::decisions::Json, TextCompletionFailure>,
    ChatChunkState,
)> {
    loop {
        if let Some(chunk) = state.pending.pop_front() {
            return Some((Ok(chunk), state));
        }
        if state.finished {
            return None;
        }
        match state.bytes.next().await {
            Some(Ok(bytes)) => {
                let parsed = state
                    .parser
                    .push(&bytes)
                    .map_err(|source| internal_failure(&source.to_string()))
                    .and_then(|events| {
                        decode_chat_events(&events, &state.model, state.include_usage)
                            .map_err(|error| internal_failure(&error.to_string()))
                    });
                if let Err(error) = enqueue(&mut state, parsed) {
                    state.finished = true;
                    return Some((Err(error), state));
                }
            }
            Some(Err(error)) => {
                state.finished = true;
                return Some((Err(internal_failure(&error.to_string())), state));
            }
            None => {
                state.finished = true;
                let decoded =
                    decode_chat_events(&state.parser.finish(), &state.model, state.include_usage)
                        .map_err(|error| internal_failure(&error.to_string()));
                if let Err(error) = enqueue(&mut state, decoded) {
                    return Some((Err(error), state));
                }
            }
        }
    }
}

fn enqueue(
    state: &mut ChatChunkState,
    parsed: Result<Vec<betterlite_models::decisions::Json>, TextCompletionFailure>,
) -> Result<(), TextCompletionFailure> {
    for chunk in parsed? {
        state.pending.push_back(chunk);
    }
    Ok(())
}

fn bridge_failure(
    failure: betterlite_openai::text_completion_execution::failure::CompletionFailure,
) -> TextCompletionFailure {
    TextCompletionFailure {
        status: failure.status,
        error_type: failure.error_type,
        param: failure.param,
        message: failure.message,
    }
}

fn stream_failure(source: serde_json::Error) -> TextCompletionFailure {
    TextCompletionFailure {
        status: 500,
        error_type: None,
        param: None,
        message: source.to_string(),
    }
}

fn internal_failure(message: &str) -> TextCompletionFailure {
    TextCompletionFailure {
        status: 500,
        error_type: Some(String::from("internal_server_error")),
        param: None,
        message: message.to_owned(),
    }
}

// the chat route's status, type and param for a failure the text provider answered with
fn chat_call_failure(
    error: CallError<ChatError>,
    calls: &CompletionCalls,
) -> TextCompletionFailure {
    match error {
        CallError::Route(route) => {
            let failure = route.failure(&calls.state.executor.redactor);
            TextCompletionFailure {
                status: failure.status,
                error_type: Some(failure.error_type),
                param: failure.param,
                message: route.to_string(),
            }
        }
        CallError::Operation { source } => internal_failure(&source.to_string()),
        CallError::Mismatched => internal_failure("the provider answered a different operation"),
    }
}

fn routed_call_id(headers: &HeaderMap) -> String {
    headers
        .get("x-litellm-call-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| (1..=MAX_CALL_ID_LENGTH).contains(&value.len()))
        .map_or_else(|| Uuid::new_v4().to_string(), str::to_owned)
}

async fn completions(
    State(state): State<CompletionState>,
    path_model: Option<Path<String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let call_id = routed_call_id(&headers);
    let request = match CompletionRequest::parse(&body) {
        Ok(request) => request,
        Err(error) => {
            let mut include = state.include_call_id;
            // LiteLLM's _read_request_body reads an empty body as {} and never builds a call id for a body it cannot parse
            if let CompletionRequestError::Body { source } = &error {
                include &= body.is_empty() || !(source.is_syntax() || source.is_eof());
            }
            let answer = FailureAnswer {
                status: 400,
                error_type: Some(String::from("invalid_request_error")),
                param: None,
                message: error.to_string(),
            };
            return render(&answer, &call_id, include);
        }
    };
    let model = path_model
        .map(|Path(model)| model)
        .or_else(|| request.model.clone())
        .unwrap_or_else(|| String::from("None"));
    let Ok(deployment) = pick_deployment(&state.state.executor, &model) else {
        let missing = DispatchError::MissingDeployment { model_id: model };
        return render(&failure_answer(missing), &call_id, state.include_call_id);
    };
    execute(&state, request, deployment, &call_id, started).await
}

async fn execute(
    state: &CompletionState,
    request: CompletionRequest,
    deployment: &ChatDeployment,
    call_id: &str,
    started: Instant,
) -> Response {
    let asked_model = request.model.clone();
    let endpoint = state.execution.endpoint(deployment).await;
    let operation = operation(state, request, deployment, call_id);
    let context = DispatchContext::start(CallInputs {
        now: SystemTime::now,
        id: Uuid::new_v4,
        deadline: None,
        privacy: false,
    });
    match state.dispatch.execute(operation, &context).await {
        Ok(CallResult::TextCompletion(mut result)) => {
            // common_request_processing._override_openai_response_model answers the model the client named
            if let Some(model) = asked_model {
                result.response.model = Some(model);
            }
            match endpoint {
                // the completion handler leaves get_custom_headers no provider header, so a text answer has only x-litellm-*
                CompletionEndpoint::Text => {
                    let headers = success_headers(
                        &HeaderMap::new(),
                        deployment,
                        call_id,
                        result.upstream_time,
                        started.elapsed(),
                    );
                    (headers, Json(result.response)).into_response()
                }
                // a chat answer keeps its provider headers, which the route repeats with llm_provider-
                CompletionEndpoint::Chat => {
                    let headers = success_headers(
                        &result.upstream_headers,
                        deployment,
                        call_id,
                        result.upstream_time,
                        started.elapsed(),
                    );
                    match text_answer_body(&result.response) {
                        Ok(body) => json_response(headers, body),
                        Err(failure) => {
                            let answer = FailureAnswer {
                                status: failure.status,
                                error_type: failure.error_type,
                                param: failure.param,
                                message: failure.message,
                            };
                            render(&answer, call_id, state.include_call_id)
                        }
                    }
                }
            }
        }
        Ok(CallResult::TextStream(result)) => {
            streaming::render(result, deployment, call_id, started)
        }
        Ok(_) => {
            let unbuilt = DispatchError::NotBuilt {
                entry: "text_stream",
            };
            render(&failure_answer(unbuilt), call_id, state.include_call_id)
        }
        Err(error) => render(&failure_answer(error), call_id, state.include_call_id),
    }
}

// the chat endpoint's text answer is written by the bridge, so the route adds its own content type
fn json_response(mut headers: HeaderMap, body: Vec<u8>) -> Response {
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    (headers, body).into_response()
}

fn operation(
    state: &CompletionState,
    request: CompletionRequest,
    deployment: &ChatDeployment,
    call_id: &str,
) -> Operation {
    let calls = Arc::new(CompletionCalls {
        state: Arc::clone(&state.state),
        execution: Arc::clone(&state.execution),
    });
    let streaming = request.stream == Some(true);
    let call = TextCompletionRequest {
        call_id: call_id.to_owned(),
        model_id: deployment.model_id.clone(),
        request: Box::new(request),
        dispatch: Arc::clone(&state.dispatch),
        calls,
    };
    if streaming {
        Operation::TextStream(call)
    } else {
        Operation::TextCompletion(call)
    }
}
