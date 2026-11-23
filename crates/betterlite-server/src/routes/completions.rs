use std::sync::Arc;
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::header::CONTENT_TYPE;
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
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::completion_chat_bridge::CompletionEndpoint;
use betterlite_openai::completion_chat_bridge::answer::text_answer_body;
use betterlite_openai::text_completion_execution::CompletionExecutor;
use betterlite_openai::text_completion_request::{CompletionRequest, CompletionRequestError};
use betterlite_provider::anthropic_dispatch::gateway::runtime::{AnthropicRuntime, RuntimeError};
use betterlite_tokenizer::count::Counter;
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::pre_call::rewrites::resolve_call_id;
use crate::state::ServerState;

use super::chat::headers::success_headers;

use self::bridge::{chat_bridge, text_answer, text_stream};
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
        counter: None,
        anthropic: state.anthropic_runtime.runtime.clone(),
    })
}

#[derive(Clone)]
struct CompletionState {
    state: Arc<ServerState>,
    execution: Arc<CompletionExecutor>,
    dispatch: Arc<Dispatch>,
    include_call_id: bool,
    // the token counter the application shares with the chat route, so an output-limit reply counts as it does
    counter: Option<Arc<Counter>>,
    // the one runtime the application built; a configuration it cannot load fails at startup, so the route builds none
    anthropic: Result<Arc<AnthropicRuntime>, Arc<RuntimeError>>,
}

impl CompletionState {
    // the per-request handle the route's dispatch calls run through
    fn calls(&self) -> Arc<CompletionCalls> {
        Arc::new(CompletionCalls {
            state: Arc::clone(&self.state),
            execution: Arc::clone(&self.execution),
            counter: self.counter.clone(),
            anthropic: self.anthropic.clone(),
        })
    }
}

// one per request: LiteLLM's completion route answers a failure with the call id header alone, never with the provider's
struct CompletionCalls {
    state: Arc<ServerState>,
    execution: Arc<CompletionExecutor>,
    counter: Option<Arc<Counter>>,
    anthropic: Result<Arc<AnthropicRuntime>, Arc<RuntimeError>>,
}

impl CompletionCalls {
    // the runtime the application built; startup already refused a configuration whose runtime could not load
    fn anthropic(&self) -> Result<&Arc<AnthropicRuntime>, TextCompletionFailure> {
        self.anthropic
            .as_ref()
            .map_err(|error| TextCompletionFailure::internal(&error.as_ref().to_string()))
    }
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
        chat_bridge(self, &call, mode).await
    }
}

async fn completions(
    State(mut state): State<CompletionState>,
    counter: Option<Extension<Arc<Counter>>>,
    path_model: Option<Path<String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    state.counter = counter.map(|Extension(shared)| shared);
    let started = Instant::now();
    let call_id = resolve_call_id(
        headers
            .get("x-litellm-call-id")
            .and_then(|value| value.to_str().ok()),
    );
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
    let operation = text_operation(request, state.calls(), state, deployment, call_id);
    let context = DispatchContext::start(CallInputs {
        now: SystemTime::now,
        id: Uuid::new_v4,
        deadline: None,
        privacy: false,
    });
    let answered = state.dispatch.execute(operation, &context).await;
    let mut result = match answered {
        Ok(CallResult::TextCompletion(completion)) => completion,
        Ok(CallResult::TextStream(stream)) => {
            return streaming::render(stream, deployment, call_id, started);
        }
        Ok(_) => {
            let unbuilt = DispatchError::NotBuilt {
                entry: "text_stream",
            };
            return render(&failure_answer(unbuilt), call_id, state.include_call_id);
        }
        Err(error) => return render(&failure_answer(error), call_id, state.include_call_id),
    };
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

// the dispatch operation for the route's parsed request: a stream or a buffered completion
fn text_operation(
    request: CompletionRequest,
    calls: Arc<CompletionCalls>,
    state: &CompletionState,
    deployment: &ChatDeployment,
    call_id: &str,
) -> Operation {
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
            };
            render(&answer, call_id, state.include_call_id)
        }
    }
}

// the chat endpoint's text answer is written by the bridge, so the route adds its own content type
fn json_response(mut headers: HeaderMap, body: Vec<u8>) -> Response {
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    (headers, body).into_response()
}
