use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use axum::Json;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::settings::JsonValue;
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_router::request_scheduler::error::SchedulerError;
use betterlite_router::request_scheduler::flow::FlowItem;
use betterlite_router::request_scheduler::schedule::{ScheduleOutcome, ScheduledCall};
use betterlite_router::request_scheduler::service::RequestScheduler;
use betterlite_router::router_core::headers;
use betterlite_router::routing_timeouts::error::TimeoutError;
use thiserror::Error;

use super::super::state::{QueueCompletion, QueueFailureHook, QueueRouter, timeout_plan};
use super::request::QueueRequest;
use crate::auth::auth_resource_grants::denial_response;
use crate::sse_data_generator::types::StreamSource;

mod tests;

pub(super) const PRIORITY_HEADER: &str = "x-litellm-priority";
const HIDDEN_PARAMS: &str = "_hidden_params";

#[derive(Debug, Error)]
pub(super) enum QueueError {
    #[error("{0}")]
    Message(String),
    #[error("the request carries no priority between 0 and 255")]
    Priority,
    #[error("No models configured on proxy")]
    NoRouter,
    #[error("the queued completion answered no response object")]
    EmptyAnswer,
    #[error(transparent)]
    Scheduler(#[from] SchedulerError),
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
}

impl QueueError {
    // every non-HTTPException becomes "Authentication Error, ..." with a 400, and an HTTPException keeps its status
    pub(super) fn exception(&self) -> ProxyException {
        match self {
            Self::Message(text) => auth_error(
                format!("Authentication Error, {text}"),
                StatusCode::BAD_REQUEST,
            ),
            Self::Priority | Self::EmptyAnswer | Self::Timeout(_) => auth_error(
                format!("Authentication Error, {self}"),
                StatusCode::BAD_REQUEST,
            ),
            Self::NoRouter => auth_error(
                CommonProxyErrors::NoLlmRouter.as_str().to_owned(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            Self::Scheduler(error) => scheduler_exception(error),
        }
    }
}

fn auth_error(message: String, status: StatusCode) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message,
        error_type: ProxyErrorTypes::AuthError.to_string(),
        param: Some("None".to_owned()),
        code: ExceptionCode::Status(i64::from(status.as_u16())),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

// a full queue keeps the bound's own 429, and everything else is LiteLLM's 400 "Authentication Error"
fn scheduler_exception(error: &SchedulerError) -> ProxyException {
    if let SchedulerError::QueueFull { .. } = error {
        return auth_error(error.to_string(), StatusCode::TOO_MANY_REQUESTS);
    }
    if let SchedulerError::Provider { error: cause, .. } = error {
        return auth_error(
            format!("Authentication Error, {}", class_message(cause)),
            StatusCode::BAD_REQUEST,
        );
    }
    auth_error(
        format!("Authentication Error, {error}"),
        StatusCode::BAD_REQUEST,
    )
}

pub(super) struct QueuedCompletion<'call> {
    pub(super) completion: &'call dyn QueueCompletion,
    pub(super) caller: &'call Principal,
    pub(super) data: &'call JsonMap,
    pub(super) stream: bool,
    pub(super) source: Mutex<Option<StreamSource>>,
}

impl QueuedCompletion<'_> {
    pub(super) fn take_source(&self) -> Option<StreamSource> {
        self.source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

#[async_trait]
impl ScheduledCall for QueuedCompletion<'_> {
    fn healthy_deployments(&self) -> usize {
        self.completion.healthy_deployments(&model_name(self.data))
    }

    async fn call(&self) -> Result<headers::Response, Box<ErrorClass>> {
        let answer = self
            .completion
            .complete(self.caller, self.data, self.stream)
            .await?;
        if let Some(source) = answer.stream {
            *self.source.lock().unwrap_or_else(PoisonError::into_inner) = Some(source);
        }
        Ok(answer.response)
    }
}

pub(super) fn scheduler_item(
    scheduler: &RequestScheduler,
    data: &JsonMap,
) -> Result<FlowItem, QueueError> {
    Ok(FlowItem {
        priority: priority(data)?,
        request_id: scheduler.next_request_id()?,
        model: model_name(data),
    })
}

fn priority(data: &JsonMap) -> Result<u8, QueueError> {
    data.get("priority")
        .and_then(JsonValue::as_u64)
        .and_then(|value| u8::try_from(value).ok())
        .ok_or(QueueError::Priority)
}

fn model_name(data: &JsonMap) -> String {
    data.get("model")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub(super) async fn non_streaming(
    request: QueueRequest<'_>,
    router: &QueueRouter,
    data: &JsonMap,
) -> Result<Response, QueueError> {
    let adapter = QueuedCompletion {
        completion: router.completion.as_ref(),
        caller: request.caller,
        data,
        stream: false,
        source: Mutex::new(None),
    };
    let item = scheduler_item(router.scheduler.as_ref(), data)?;
    let plan = timeout_plan(&request.queue.timeouts, false)?;
    let outcome = router.scheduler.schedule(&item, &adapter, &plan).await?;
    answer(outcome)
}

// Router.schedule_acompletion's dict response, with the priority header the route reads from data["priority"]
fn answer(outcome: ScheduleOutcome) -> Result<Response, QueueError> {
    let headers::Response::Object(mut body) = outcome.response else {
        return Err(QueueError::EmptyAnswer);
    };
    body.shift_remove(HIDDEN_PARAMS);
    let mut response = Json(DecisionJson::Object(body)).into_response();
    let priority = outcome.priority.to_string();
    let Ok(value) = HeaderValue::from_str(&priority) else {
        return Err(QueueError::EmptyAnswer);
    };
    response.headers_mut().insert(PRIORITY_HEADER, value);
    Ok(response)
}

pub(super) async fn report(
    hook: Option<&dyn QueueFailureHook>,
    caller: &Principal,
    data: &JsonMap,
    exception: ProxyException,
) -> Response {
    let answered = match hook {
        Some(callback) => callback
            .post_call_failure(caller, &exception, data)
            .await
            .unwrap_or(exception),
        None => exception,
    };
    denial_response(&answered)
}
