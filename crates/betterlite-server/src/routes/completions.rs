use std::sync::Arc;
use std::time::{Instant, SystemTime};

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
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::completion_chat_bridge::answer::text_answer_body;
use betterlite_openai::completion_chat_bridge::{CompletionEndpoint, chat_request};
use betterlite_openai::text_completion_execution::CompletionExecutor;
use betterlite_openai::text_completion_request::{CompletionRequest, CompletionRequestError};
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::server::chat_setup::provider_call::answer_chat;
use crate::state::ServerState;

use super::chat::MAX_CALL_ID_LENGTH;
use super::chat::headers::success_headers;

use self::bridge::{
    bridge_failure, chat_call_failure, chat_stream_answer, converted, json_response, text_answer,
    text_result, text_stream,
};
use self::error::{FailureAnswer, failure_answer, render};

mod bridge;
mod error;
mod read_through;
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
            bundled_catalog: Arc::clone(&executor.bundled_catalog),
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
        text_answer(self, &call).await
    }

    async fn endpoint(&self, deployment: &ChatDeployment) -> CompletionEndpoint {
        self.execution.endpoint(deployment).await
    }

    async fn stream(
        &self,
        call: TextCompletionCallInput<'_>,
    ) -> Result<TextStreamAnswer, TextCompletionFailure> {
        text_stream(self, &call).await
    }

    async fn chat(
        &self,
        call: TextCompletionCallInput<'_>,
        mode: ChatCallMode,
    ) -> Result<ChatCallAnswer, TextCompletionFailure> {
        let chat = chat_request(call.request).map_err(bridge_failure)?;
        let control = ControlOptions::default();
        match mode {
            ChatCallMode::Buffered => {
                let (completion, headers, time) =
                    answer_chat(&self.state.executor, &chat, &call.deployment, control)
                        .await
                        .map_err(|error| chat_call_failure(error, self))?;
                Ok(ChatCallAnswer::Buffered(text_result(converted(
                    completion, headers, time,
                )?)))
            }
            ChatCallMode::Streamed => {
                let started = Instant::now();
                let upstream = streaming::provider_stream(
                    &self.state.executor,
                    &call.deployment,
                    &chat,
                    control,
                    streaming::StreamUsage::Text,
                )
                .await
                .map_err(|error| chat_call_failure(error, self))?;
                let options = call.request.stream_options.as_ref();
                let include_usage = options.is_some_and(|chosen| chosen.include_usage);
                Ok(ChatCallAnswer::Streamed(chat_stream_answer(
                    upstream,
                    call.request.model.as_deref(),
                    include_usage,
                    started.elapsed(),
                )))
            }
        }
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
            let answer = FailureAnswer::plain(400, "invalid_request_error", error.to_string());
            return render(&answer, &call_id, include);
        }
    };
    let model = path_model
        .map(|Path(model)| model)
        .or_else(|| request.model.clone());
    let deployment = match read_through::deployment_or_answer(&state.state, model.as_deref()).await
    {
        Ok(deployment) => deployment,
        Err(answer) => return render(&answer, &call_id, state.include_call_id),
    };
    execute(&state, request, &deployment, &call_id, started).await
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
            // the completion handler leaves get_custom_headers no provider header, so a text answer has only x-litellm-*
            if matches!(endpoint, CompletionEndpoint::Text) {
                let headers = success_headers(
                    &HeaderMap::new(),
                    deployment,
                    call_id,
                    result.upstream_time,
                    started.elapsed(),
                );
                return (headers, Json(result.response)).into_response();
            }
            chat_response(&result, deployment, call_id, state, started)
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

// A chat answer keeps its provider headers, which the route repeats with llm_provider-
fn chat_response(
    result: &TextCompletionResult,
    deployment: &ChatDeployment,
    call_id: &str,
    state: &CompletionState,
    started: Instant,
) -> Response {
    match text_answer_body(&result.response) {
        Ok(body) => {
            let headers = success_headers(
                &result.upstream_headers,
                deployment,
                call_id,
                result.upstream_time,
                started.elapsed(),
            );
            json_response(headers, body)
        }
        Err(failure) => {
            let answer = FailureAnswer {
                status: failure.status,
                error_type: failure.error_type,
                param: failure.param,
                message: failure.message,
                provider_specific_fields: None,
            };
            render(&answer, call_id, state.include_call_id)
        }
    }
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
