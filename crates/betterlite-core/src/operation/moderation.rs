use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::envelope::WireField;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::moderation_execution::ModerationError;
use betterlite_openai::moderation_request::{
    ModerationRequest as ClientModerationRequest, ModerationRequestError,
};
use betterlite_openai::moderation_response::{ModerationResponse, ModerationResponseError};
use betterlite_redact::secret::Redactor;
use http::HeaderMap;
use serde_json::Value;

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
    fn plain(status: u16, error_type: &str, message: String) -> Self {
        Self {
            status,
            error_type: Some(error_type.to_owned()),
            param: None,
            code: None,
            message,
        }
    }
}

#[must_use]
pub fn route_error(error: &ModerationError, redactor: &Redactor) -> ModerationFailure {
    let message = redactor.redact_internal_details(&error.to_string());
    match error {
        ModerationError::Provider { response } => {
            return provider_route(response, redactor);
        }
        ModerationError::Request(ModerationRequestError::Body { .. })
        | ModerationError::Request(ModerationRequestError::Unsupported { .. }) => {
            return ModerationFailure::plain(400, "invalid_request_error", message);
        }
        ModerationError::Response(ModerationResponseError::Decimal { .. }) => {
            return ModerationFailure {
                status: 502,
                error_type: Some(String::from("bad_gateway")),
                param: Some(String::from("category_scores")),
                code: Some(String::from("decimal_out_of_range")),
                message,
            };
        }
        ModerationError::MissingApiKey => {
            return ModerationFailure::plain(401, "authentication_error", message);
        }
        ModerationError::UnsupportedProvider { .. } => {
            return ModerationFailure::plain(501, "internal_server_error", message);
        }
        // litellm.Timeout keeps openai.APITimeoutError's absent type and param, and the proxy answers 408
        ModerationError::Http {
            source: HttpError::Timeout,
        } => {
            return ModerationFailure {
                status: 408,
                error_type: None,
                param: None,
                code: None,
                message,
            };
        }
        // openai.APIConnectionError also carries no type or param
        ModerationError::Http { .. } => {
            return ModerationFailure {
                status: 500,
                error_type: None,
                param: None,
                code: None,
                message,
            };
        }
        ModerationError::Request(ModerationRequestError::Encode { .. })
        | ModerationError::Response(ModerationResponseError::Decode { .. })
        | ModerationError::Endpoint { .. } => {}
    }
    ModerationFailure::plain(500, "internal_server_error", message)
}

// proxy_server.py's moderation handler answers getattr(e, "type"/"param"/"status_code") with no openai_error_type fallback
fn provider_route(response: &UpstreamResponse, redactor: &Redactor) -> ModerationFailure {
    let class = dispatch(
        &DispatchInput::Raw(Box::new(raw_exception(response))),
        redactor,
    );
    let metadata = class.metadata();
    ModerationFailure {
        status: metadata.status_code,
        error_type: wire(metadata.error_type),
        param: wire(metadata.param),
        code: Some(metadata.code),
        message: redactor.redact_internal_details(&format!(
            "the provider answered with status {}",
            response.status
        )),
    }
}

// getattr(e, field, "None"): a missing attribute answers the literal string, an attribute set to null answers null
fn wire(field: WireField) -> Option<String> {
    match field {
        WireField::Missing(_) => Some(String::from("None")),
        WireField::Null => None,
        WireField::Text(text) => Some(text),
    }
}

// the openai SDK's APIStatusError: str(e) is "Error code: {status} - {body}" and e.body is the error object, or the whole body without one
fn raw_exception(response: &UpstreamResponse) -> RawException {
    RawException {
        model: String::new(),
        custom_llm_provider: String::from("openai"),
        error_str: format!(
            "Error code: {} - {}",
            response.status,
            str::from_utf8(&response.body).unwrap_or_default()
        ),
        status_code: Some(response.status),
        status_code_is_synthesized: false,
        message: None,
        body: sdk_body(&response.body),
        exception_type: String::new(),
    }
}

// the SDK's _make_status_error unwraps {"error": ...} before APIError reads type, param and code off it
fn sdk_body(bytes: &[u8]) -> Option<RawExceptionBody> {
    let data: Value = serde_json::from_slice(bytes).ok()?;
    let error = data.get("error").unwrap_or(&data).as_object()?;
    let text = |name: &str| error.get(name).and_then(Value::as_str).map(str::to_owned);
    Some(RawExceptionBody {
        kind: text("type"),
        param: text("param"),
        code: text("code"),
        message: text("message"),
        error: None,
    })
}

mod tests;
