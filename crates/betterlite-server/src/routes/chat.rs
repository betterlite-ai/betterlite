pub(crate) mod headers;
pub(crate) mod prompt_execution;
mod streaming;

use streaming::{ChatCalls, render_stream};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::{CONTENT_TYPE, HeaderValue};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_context::call_context::{CallContext as DispatchContext, CallInputs};
use betterlite_core::dispatch::error::DispatchError;
use betterlite_core::dispatch::log::TracingCallLog;
use betterlite_core::dispatch::{Dispatch, NoRules};
use betterlite_core::operation::chat::{
    ChatCall, ChatFailure, ChatRequest as DispatchChatRequest, ChatResult,
};
use betterlite_core::operation::{CallResult, Operation};
use betterlite_error::envelope::EnvelopeError;
use betterlite_error::normalize::class::Class;
use betterlite_error::normalize::provider::provider_facts;
use betterlite_error::normalize::{ErrorFacts, Normalizer};
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::chat::{ChatRequest, RequestError};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use uuid::Uuid;

use crate::routing::simple_shuffle::pick_deployment;
use crate::server::{failure, plain_failure};

use headers::success_headers;

// LiteLLM's MAX_LITELLM_CALL_ID_LENGTH
const MAX_CALL_ID_LENGTH: usize = 256;

pub(crate) fn chat_routes_with_call_id(
    executor: Arc<ChatExecutor>,
    include_call_id: bool,
) -> Result<Router, EnvelopeError> {
    let errors = Arc::new(ErrorLayer::new()?);
    let calls: Arc<dyn ChatCall> = Arc::new(ChatCalls {
        executor: Arc::clone(&executor),
    });
    let log = Arc::new(TracingCallLog);
    let dispatch = Arc::new(Dispatch::new(
        executor.deployments.clone(),
        Arc::new(NoRules),
        log,
    ));
    let router = Router::new()
        .route("/chat/completions", post(chat_completion))
        .route("/v1/chat/completions", post(chat_completion))
        .route("/engines/{model}/chat/completions", post(chat_completion))
        .route(
            "/openai/deployments/{model}/chat/completions",
            post(chat_completion),
        );
    Ok(router.with_state(ChatRouteState {
        executor,
        calls,
        dispatch,
        include_call_id,
        errors,
    }))
}

pub(crate) struct ErrorLayer {
    pub(crate) redactor: Redactor,
    pub(crate) normalizer: Normalizer,
}

impl ErrorLayer {
    pub(crate) fn new() -> Result<Self, EnvelopeError> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
        let normalizer = Normalizer::new()?;
        Ok(Self {
            redactor,
            normalizer,
        })
    }
}

#[derive(Clone)]
struct ChatRouteState {
    executor: Arc<ChatExecutor>,
    calls: Arc<dyn ChatCall>,
    dispatch: Arc<Dispatch>,
    include_call_id: bool,
    errors: Arc<ErrorLayer>,
}

#[derive(Clone)]
pub(crate) struct CallContext<'id> {
    pub(crate) id: &'id str,
    pub(crate) include_in_body: bool,
    pub(crate) started: Instant,
    pub(crate) errors: &'id ErrorLayer,
}

async fn chat_completion(
    State(state): State<ChatRouteState>,
    path_model: Option<Path<String>>,
    request_headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let call_id = request_headers
        .get("x-litellm-call-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| (1..=MAX_CALL_ID_LENGTH).contains(&value.len()))
        .map_or_else(|| Uuid::new_v4().to_string(), str::to_owned);
    let mut call = CallContext {
        id: &call_id,
        include_in_body: state.include_call_id,
        started,
        errors: &state.errors,
    };
    let request = match ChatRequest::parse(&body) {
        Ok(request) => request,
        Err(source) => {
            if let RequestError::Body { source: json } = &source {
                // LiteLLM's _read_request_body treats an empty body as {} instead of raising.
                call.include_in_body &= body.is_empty() || !(json.is_syntax() || json.is_eof());
            }
            let error = ChatError::from(source);
            log_unrouted_failure(&error, &call);
            return failure(&error, &call, None);
        }
    };
    let model = path_model
        .map(|Path(model)| model)
        .or_else(|| request.model.clone());
    let deployment = match pick_deployment(&state.executor, model.as_deref().unwrap_or("None")) {
        Ok(deployment) => deployment,
        Err(error) => {
            log_unrouted_failure(&error, &call);
            return failure(&error, &call, None);
        }
    };
    execute_chat(&state, &request, deployment, &call).await
}

// only the dispatch's own terminal record covers a routed call; this covers one that never got there
fn log_unrouted_failure(error: &ChatError, call: &CallContext<'_>) {
    let message = call
        .errors
        .redactor
        .redact_internal_details(&error.to_string());
    tracing::warn!(message, "chat request failed before dispatch");
}

async fn execute_chat(
    state: &ChatRouteState,
    request: &ChatRequest,
    deployment: &ChatDeployment,
    call: &CallContext<'_>,
) -> Response {
    let operation = Operation::Chat(DispatchChatRequest {
        call_id: call.id.to_owned(),
        model_id: deployment.model_id.clone(),
        request: Box::new(request.clone()),
        calls: Arc::clone(&state.calls),
    });
    let context = DispatchContext::start(CallInputs {
        now: SystemTime::now,
        id: Uuid::new_v4,
        deadline: None,
        privacy: false,
    });
    match state.dispatch.execute(operation, &context).await {
        Ok(CallResult::Chat(ChatResult::Completion(answer))) => {
            let response_headers = success_headers(
                &answer.upstream_headers,
                deployment,
                call.id,
                answer.upstream_time,
                call.started.elapsed(),
            );
            let json = [(CONTENT_TYPE, HeaderValue::from_static("application/json"))];
            (response_headers, json, answer.body).into_response()
        }
        Ok(CallResult::Chat(ChatResult::Stream(answer))) => {
            render_stream(answer, deployment, request, call)
        }
        Ok(_) => dispatch_failure(
            DispatchError::NotBuilt { entry: "chat" },
            call,
            Some(deployment),
        ),
        Err(error) => dispatch_failure(error, call, Some(deployment)),
    }
}

fn dispatch_failure(
    error: DispatchError,
    call: &CallContext<'_>,
    deployment: Option<&ChatDeployment>,
) -> Response {
    match error {
        DispatchError::Chat { source } => chat_failure(source, call, deployment),
        DispatchError::Rules { source } => internal_failure(&source.to_string(), call, deployment),
        DispatchError::DeadlineExpired { .. } => failure(&ChatError::Timeout, call, deployment),
        DispatchError::MissingDeployment { model_id } => failure(
            &ChatError::UnknownModel { model: model_id },
            call,
            deployment,
        ),
        DispatchError::Rejected {
            status,
            error_type,
            param,
            message,
            ..
        } => plain_failure(
            call,
            deployment,
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            (&error_type, param),
            message,
        ),
        DispatchError::Embedding { .. }
        | DispatchError::NotBuilt { .. }
        | DispatchError::UnknownOperation { .. }
        | DispatchError::MismatchedOperation { .. } => {
            internal_failure("the chat call could not be dispatched", call, deployment)
        }
    }
}

fn chat_failure(
    chat_error: ChatFailure,
    call: &CallContext<'_>,
    deployment: Option<&ChatDeployment>,
) -> Response {
    match chat_error {
        ChatFailure::Route(route) => failure(&route, call, deployment),
        ChatFailure::InvalidControl { source } => plain_failure(
            call,
            deployment,
            StatusCode::BAD_REQUEST,
            ("invalid_request_error", Some(source.param().to_owned())),
            source.to_string(),
        ),
        ChatFailure::Internal { message } => internal_failure(&message, call, deployment),
        ChatFailure::Mismatched => internal_failure(
            "the provider answered a different operation",
            call,
            deployment,
        ),
    }
}

fn internal_failure(
    message: &str,
    call: &CallContext<'_>,
    deployment: Option<&ChatDeployment>,
) -> Response {
    let redacted = call.errors.redactor.redact_internal_details(message);
    plain_failure(
        call,
        deployment,
        StatusCode::INTERNAL_SERVER_ERROR,
        ("internal_server_error", None),
        redacted,
    )
}

pub(crate) fn log_normalized_error(
    error: &ChatError,
    message: &str,
    status: u16,
    layer: &ErrorLayer,
) {
    let facts = if let ChatError::Provider { response } = error {
        provider_facts(response.status, &response.body, &layer.redactor)
    } else {
        let class = match error {
            ChatError::Timeout => Class::Timeout,
            ChatError::Http { .. } => Class::APIConnectionError,
            ChatError::UnknownModel { .. }
            | ChatError::Request { .. }
            | ChatError::Streaming
            | ChatError::UnsupportedProvider { .. }
            | ChatError::MissingApiKey
            | ChatError::Provider { .. }
            | ChatError::Encode { .. }
            | ChatError::Decode { .. }
            | ChatError::TooLarge => Class::Other,
        };
        ErrorFacts {
            proxy_type: None,
            message: message.to_owned(),
            class,
            status_code: Some(status),
            rate_limit_type: None,
        }
    };
    let normalized_error = layer.normalizer.normalize(Some(&facts));
    tracing::warn!(normalized_error, "chat request failed");
}

mod tests;
