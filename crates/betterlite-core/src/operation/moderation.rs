use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::moderation_execution::ModerationError;
use betterlite_openai::moderation_request::{
    ModerationRequest as ClientModerationRequest, ModerationRequestError,
};
use betterlite_openai::moderation_response::{ModerationResponse, ModerationResponseError};
use betterlite_openai::text_completion_transport::sdk::{SdkError, status_error};
use betterlite_redact::secret::Redactor;
use http::HeaderMap;

use crate::dispatch::Dispatch;

pub struct ModerationRequest {
    pub call_id: String,
    pub model_id: String,
    pub request: Box<ClientModerationRequest>,
    // the dispatch arm B014 registered takes no Dispatch, so the request hands its entry the pipeline it belongs to
    pub dispatch: Arc<Dispatch>,
    pub calls: Arc<dyn ModerationCall>,
}

pub struct ModerationCallInput<'call> {
    pub deployment: ChatDeployment,
    pub request: &'call ClientModerationRequest,
}

// the provider call sits behind this seam, so the pipeline stays in betterlite-core
#[async_trait]
pub trait ModerationCall: Send + Sync {
    async fn moderate(
        &self,
        call: ModerationCallInput<'_>,
    ) -> Result<ModerationResult, ModerationFailure>;
}

pub struct ModerationResult {
    pub response: ModerationResponse,
    pub upstream_headers: HeaderMap,
    pub upstream_time: Duration,
}

// the status, type, param, code and message the route answers with; LiteLLM's moderation handler writes its own code
#[derive(Debug)]
pub struct ModerationFailure {
    pub status: u16,
    pub error_type: Option<String>,
    pub param: Option<String>,
    pub code: Option<String>,
    pub message: String,
}

impl ModerationFailure {
    pub fn plain(status: u16, error_type: Option<&str>, message: &str) -> Self {
        Self {
            status,
            error_type: error_type.map(str::to_owned),
            param: None,
            code: None,
            message: message.to_owned(),
        }
    }
}

#[must_use]
pub fn route_error(error: &ModerationError, redactor: &Redactor) -> ModerationFailure {
    let message = redactor.redact_internal_details(&error.to_string());
    match error {
        ModerationError::Provider { response } => provider_route(response, redactor),
        ModerationError::Request(request) => request_route(request, &message),
        ModerationError::Response(ModerationResponseError::Decimal { .. }) => ModerationFailure {
            status: 502,
            error_type: Some(String::from("bad_gateway")),
            param: Some(String::from("category_scores")),
            code: Some(String::from("decimal_out_of_range")),
            message,
        },
        ModerationError::Response(ModerationResponseError::Decode { .. })
        | ModerationError::Endpoint { .. } => {
            ModerationFailure::plain(500, Some("internal_server_error"), &message)
        }
        ModerationError::MissingApiKey => {
            ModerationFailure::plain(401, Some("authentication_error"), &message)
        }
        ModerationError::UnsupportedProvider { .. } => {
            ModerationFailure::plain(501, Some("internal_server_error"), &message)
        }
        ModerationError::Http { source } => transport_route(source, &message),
    }
}

fn request_route(error: &ModerationRequestError, message: &str) -> ModerationFailure {
    match error {
        ModerationRequestError::Body { .. } | ModerationRequestError::Unsupported { .. } => {
            ModerationFailure::plain(400, Some("invalid_request_error"), message)
        }
        ModerationRequestError::Encode { .. } => {
            ModerationFailure::plain(500, Some("internal_server_error"), message)
        }
    }
}

// litellm.Timeout keeps openai.APITimeoutError's absent type and param, and the proxy answers 408
fn transport_route(error: &HttpError, message: &str) -> ModerationFailure {
    let status = match error {
        HttpError::Timeout => 408,
        // openai.APIConnectionError also carries no type or param
        HttpError::Required { .. }
        | HttpError::DotSegment { .. }
        | HttpError::UnsupportedProxyScheme { .. }
        | HttpError::InvalidProxyUrl { .. }
        | HttpError::Client(_)
        | HttpError::Request(_)
        | HttpError::TooLarge { .. } => 500,
    };
    ModerationFailure::plain(status, None, message)
}

// proxy_server.py's moderation handler answers the SDK exception's own type and param, and str(status_code) as the code
fn provider_route(response: &UpstreamResponse, redactor: &Redactor) -> ModerationFailure {
    match status_error(response.status, response.headers.clone(), &response.body) {
        SdkError::Status(status) => ModerationFailure {
            status: response.status,
            error_type: status.error_type,
            param: status.param,
            code: Some(response.status.to_string()),
            message: redactor.redact_internal_details(&format!(
                "the provider answered with status {}",
                response.status
            )),
        },
        // status_error builds a Status error for every response
        other @ (SdkError::Decode(_) | SdkError::Sse(_) | SdkError::Other { .. }) => {
            ModerationFailure::plain(
                500,
                Some("internal_server_error"),
                &redactor.redact_internal_details(&other.to_string()),
            )
        }
    }
}

mod tests;
