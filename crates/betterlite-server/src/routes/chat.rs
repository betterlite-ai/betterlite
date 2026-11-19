pub(crate) mod headers;
pub(crate) mod prompt_execution;
pub(crate) mod streaming;

use streaming::{ChatCalls, render_stream};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::header::{CONTENT_TYPE, HeaderValue};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_context::call_context::{CallContext as DispatchContext, CallInputs};
use betterlite_core::dispatch::error::DispatchError;
use betterlite_core::dispatch::log::TracingCallLog;
use betterlite_core::dispatch::{Dispatch, NoRules};
use betterlite_core::operation::chat::first_message::first_message_has_content;
use betterlite_core::operation::chat::{ChatCall, ChatRequest as DispatchChatRequest, ChatResult};
use betterlite_core::operation::{CallResult, Operation};
use betterlite_error::envelope::EnvelopeError;
use betterlite_error::normalize::{ErrorFacts, Normalizer, class::Class, provider::provider_facts};
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use uuid::Uuid;

use crate::pre_call::rewrites::resolve_call_id;
use crate::server::failure;
use crate::server::render::{FailureShape, plain_failure};
use crate::state::ServerState;

use self::failure::{
    chat_failure, internal_failure, not_found_failure, parse_chat_request, routed_deployment,
};
use headers::success_headers;

// LiteLLM's MAX_LITELLM_CALL_ID_LENGTH, the longest client-supplied call id the routes accept
pub(super) const MAX_CALL_ID_LENGTH: usize = 256;

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
        Ok(Self {
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?,
            normalizer: Normalizer::new()?,
        })
    }

    pub(crate) fn log_normalized_error(&self, error: &ChatError, message: &str, status: u16) {
        let facts = if let ChatError::Provider { response } = error {
            provider_facts(response.status, &response.body, &self.redactor)
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
        let normalized_error = self.normalizer.normalize(Some(&facts));
        tracing::warn!(normalized_error, "chat request failed");
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
    server: Option<Extension<Arc<ServerState>>>,
    request_headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let header = request_headers.get("x-litellm-call-id");
    let call_id = resolve_call_id(header.and_then(|value| value.to_str().ok()));
    let mut call = CallContext {
        id: &call_id,
        include_in_body: state.include_call_id,
        started,
        errors: &state.errors,
    };
    let request = match parse_chat_request(&body, &mut call) {
        Ok(request) => request,
        Err(error) => {
            // only the dispatch's own terminal record covers a routed call; this covers one that never got there
            let message = call
                .errors
                .redactor
                .redact_internal_details(&error.to_string());
            tracing::warn!(error = %message, "chat request failed before dispatch");
            return failure(&error, &call, None);
        }
    };
    let model = path_model
        .map(|Path(model)| model)
        .or_else(|| request.model.clone());
    let deployment = match routed_deployment(&state, model.as_deref(), server).await {
        Ok(deployment) => deployment,
        Err(error) => return not_found_failure(&call, &error),
    };
    let has_content = first_message_has_content(&body);
    execute_chat(&state, &request, has_content, deployment, &call).await
}

async fn execute_chat(
    state: &ChatRouteState,
    request: &ChatRequest,
    has_content: bool,
    deployment: &ChatDeployment,
    call: &CallContext<'_>,
) -> Response {
    let operation = Operation::Chat(DispatchChatRequest {
        call_id: call.id.to_owned(),
        model_id: deployment.model_id.clone(),
        request: Box::new(request.clone()),
        first_message_has_content: has_content,
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
        DispatchError::Chat { source } => chat_failure(&source, call, deployment),
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
            FailureShape {
                status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                error_type: &error_type,
                param,
                message,
                repeat_message: false,
            },
        ),
        DispatchError::Embedding { .. }
        | DispatchError::NotBuilt { .. }
        | DispatchError::UnknownOperation { .. }
        | DispatchError::MismatchedOperation { .. } => {
            internal_failure("the chat call could not be dispatched", call, deployment)
        }
    }
}

mod failure;
mod tests;
