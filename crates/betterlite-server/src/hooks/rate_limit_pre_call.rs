use std::collections::BTreeMap;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::routes::CallType;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::body::ProxyErrorBody;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_rate_limit::fail_closed::RateLimitUnverifiableError;
use betterlite_rate_limit::limiter_assembly::selection::{LimiterChoice, select_call_type_limiter};
use betterlite_rate_limit::pre_call_hook::{
    PreCallHookError, PreCallHookRejection, PreCallHookRequest, RateLimitPreCallHook,
};
use betterlite_rate_limit::request_stash::RequestStashSlot;
use betterlite_rate_limit::token_estimation::output_cap::CapEndpoint;
use serde::Serialize;

mod tests;

pub enum LimiterSelection {
    V3(RateLimitPreCallHook),
    Legacy,
}

pub struct RateLimitPreCallGate {
    pub selection: LimiterSelection,
}

// one per proxy request, shared by the limiter, the callbacks and any retry or fallback inside that request
#[derive(Clone, Debug, Default)]
pub struct RequestRateLimitState {
    pub stash: RequestStashSlot,
}

pub struct RateLimitGateRequest<'request> {
    pub auth: &'request UserAPIKeyAuth,
    pub data: &'request mut RequestData,
    pub call_type: CallType,
    pub endpoint: CapEndpoint,
    pub state: &'request RequestRateLimitState,
}

#[derive(Debug, thiserror::Error)]
pub enum RateLimitPreCallError {
    #[error("the batch rate limiter is not registered for {call_type:?}")]
    MissingBatchLimiter { call_type: CallType },
    #[error(transparent)]
    Hook(#[from] PreCallHookError),
}

#[derive(Serialize)]
struct DetailBody<Detail> {
    detail: Detail,
}

#[derive(Serialize)]
struct ErrorDetail {
    error: String,
}

// _handle_llm_api_exception re-raises an unhandled pre-call exception as the OpenAI error object
#[derive(Serialize)]
struct ProxyErrorEnvelope {
    error: ProxyErrorBody,
}

impl RateLimitPreCallGate {
    #[must_use]
    pub fn main_limiter(&self) -> Option<&RateLimitPreCallHook> {
        match &self.selection {
            LimiterSelection::V3(limiter) => Some(limiter),
            LimiterSelection::Legacy => None,
        }
    }

    pub async fn pre_call(
        &self,
        request: RateLimitGateRequest<'_>,
    ) -> Result<(), RateLimitPreCallError> {
        let RateLimitGateRequest {
            auth,
            data,
            call_type,
            endpoint,
            state,
        } = request;
        let LimiterSelection::V3(limiter) = &self.selection else {
            return Ok(());
        };
        if select_call_type_limiter(call_type) == LimiterChoice::Batch
            && limiter.assembly.batch_limiter().is_none()
        {
            return Err(RateLimitPreCallError::MissingBatchLimiter { call_type });
        }
        limiter
            .pre_call_hook(PreCallHookRequest {
                auth,
                data,
                call_type,
                endpoint,
                stash: &state.stash,
            })
            .await?;
        Ok(())
    }
}

impl IntoResponse for RateLimitPreCallError {
    fn into_response(self) -> Response {
        match self {
            Self::MissingBatchLimiter { call_type } => {
                let message = format!("the batch rate limiter is not registered for {call_type:?}");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(DetailBody {
                        detail: ErrorDetail { error: message },
                    }),
                )
                    .into_response()
            }
            Self::Hook(error) => hook_response(error),
        }
    }
}

fn hook_response(error: PreCallHookError) -> Response {
    match error.rejection() {
        Some(PreCallHookRejection::RateLimited(rate_limited)) => {
            rate_limited.clone().into_response()
        }
        Some(PreCallHookRejection::Unverifiable(unverifiable)) => {
            unverifiable_response(unverifiable)
        }
        None => internal_error_response(error.to_string()),
    }
}

fn internal_error_response(message: String) -> Response {
    let exception = ProxyException::new(ProxyExceptionInit {
        message,
        error_type: ProxyErrorTypes::InternalServerError.to_string(),
        param: None,
        code: ExceptionCode::Status(500),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    });
    let body = ProxyErrorEnvelope {
        error: exception.to_body(),
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}

fn unverifiable_response(error: &RateLimitUnverifiableError) -> Response {
    match StatusCode::from_u16(error.status_code) {
        Ok(status) => (
            status,
            Json(DetailBody {
                detail: &error.detail,
            }),
        )
            .into_response(),
        Err(source) => internal_error_response(format!(
            "the fail-closed rate-limit rejection carries status {} that HTTP cannot carry: {source}",
            error.status_code
        )),
    }
}
