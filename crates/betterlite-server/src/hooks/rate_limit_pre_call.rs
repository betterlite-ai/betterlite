use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::routes::CallType;
use betterlite_rate_limit::fail_closed::RateLimitUnverifiableError;
use betterlite_rate_limit::limiter_assembly::selection::{LimiterChoice, select_call_type_limiter};
use betterlite_rate_limit::pre_call_hook::{
    PreCallHookError, PreCallHookRequest, RateLimitPreCallHook,
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
                error_body_response(StatusCode::SERVICE_UNAVAILABLE, message)
            }
            Self::Hook(error) => hook_response(error),
        }
    }
}

fn hook_response(error: PreCallHookError) -> Response {
    if let Some(rate_limited) = error.rate_limited() {
        return rate_limited.clone().into_response();
    }
    if let Some(unverifiable) = error.unverifiable() {
        return unverifiable_response(unverifiable);
    }
    error_body_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

fn unverifiable_response(error: &RateLimitUnverifiableError) -> Response {
    let status = StatusCode::from_u16(error.status_code).unwrap_or(StatusCode::SERVICE_UNAVAILABLE);
    (
        status,
        Json(DetailBody {
            detail: &error.detail,
        }),
    )
        .into_response()
}

fn error_body_response(status: StatusCode, message: String) -> Response {
    let detail = ErrorDetail { error: message };
    (status, Json(DetailBody { detail })).into_response()
}
