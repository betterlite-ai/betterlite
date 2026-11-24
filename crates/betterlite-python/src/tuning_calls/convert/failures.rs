use std::collections::BTreeMap;
use std::str::from_utf8;

use betterlite_core::api::fine_tuning_cancel_call::FineTuningCancelError;
use betterlite_core::api::fine_tuning_create::FineTuningCreateError;
use betterlite_core::api::fine_tuning_read::FineTuningReadError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use http::header::HeaderMap;
use pyo3::exceptions::PyValueError;

use super::super::error::{Failure, Raised, WireError};

const UNSUPPORTED_BODY: &str = "Unsupported provider";
// the messages the openai SDK raises when it cannot build the client LiteLLM asks it for
const OPENAI_KEY_MESSAGE: &str = "Missing credentials. Please pass an `api_key`, `workload_identity`, `admin_api_key`, or set the `OPENAI_API_KEY` or `OPENAI_ADMIN_KEY` environment variable.";
const AZURE_KEY_MESSAGE: &str = "Missing credentials. Please pass one of `api_key`, `azure_ad_token`, `azure_ad_token_provider`, or the `AZURE_OPENAI_API_KEY` or `AZURE_OPENAI_AD_TOKEN` environment variables.";
const AZURE_BASE_MESSAGE: &str = "Must provide one of the `base_url` or `azure_endpoint` arguments, or the `AZURE_OPENAI_ENDPOINT` environment variable";
const VERTEX_MESSAGE: &str = "the vertex_ai fine-tuning call is not implemented yet (B1384)";

pub(crate) fn provider_error(response: &UpstreamResponse) -> WireError {
    let headers = response_headers(&response.headers);
    let body = serde_json::from_slice::<Json>(&response.body).unwrap_or_else(|_source| {
        Json::Text(
            from_utf8(&response.body)
                .map(str::to_owned)
                .unwrap_or_default(),
        )
    });
    WireError::Provider {
        status: response.status,
        body,
        headers,
    }
}

pub(crate) fn response_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (name.as_str().to_owned(), text.to_owned()))
        })
        .collect()
}

// a key-scan of the provider's error object, the shape the openai client reads
fn error_message(body: &Json) -> String {
    let Json::Object(fields) = body else {
        return String::new();
    };
    let Some(Json::Object(error)) = fields.get("error") else {
        return String::new();
    };
    match error.get("message") {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::new(),
    }
}

// the class the openai SDK raises for a response status; 409 and the rest fall to APIError
pub(crate) const fn class_for(status: u16) -> &'static str {
    match status {
        400 => "BadRequestError",
        401 => "AuthenticationError",
        403 => "PermissionDeniedError",
        404 => "NotFoundError",
        422 => "UnprocessableEntityError",
        429 => "RateLimitError",
        500..=599 => "InternalServerError",
        _ => "APIError",
    }
}

fn failure(kind: &str, message: String) -> Failure {
    Failure {
        kind: kind.to_owned(),
        message,
        status_code: None,
        llm_provider: None,
        model: None,
        body: None,
        headers: BTreeMap::new(),
    }
}

pub(crate) fn provider_failure(
    status: u16,
    body: Json,
    headers: BTreeMap<String, String>,
) -> Failure {
    let message = error_message(&body);
    Failure {
        kind: class_for(status).to_owned(),
        message,
        status_code: Some(status),
        llm_provider: None,
        model: None,
        body: Some(body),
        headers,
    }
}

// LiteLLM answers an unsupported provider with a BadRequestError that carries no provider response body
fn unsupported_failure(provider: &str, message: String) -> Failure {
    let mut unsupported = failure("BadRequestError", message);
    unsupported.status_code = Some(400);
    unsupported.llm_provider = Some(provider.to_owned());
    unsupported.model = Some("n/a".to_owned());
    unsupported.body = Some(Json::Text(UNSUPPORTED_BODY.to_owned()));
    unsupported
}

pub(crate) fn wire_failure(source: WireError) -> Raised {
    let text = source.to_string();
    let payload = match source {
        WireError::Provider {
            status,
            body,
            headers,
        } => provider_failure(status, body, headers),
        WireError::MissingAzureBase => {
            return Raised::Python(Box::new(PyValueError::new_err(AZURE_BASE_MESSAGE)));
        }
        WireError::MissingOpenAiKey => failure("Exception", String::from(OPENAI_KEY_MESSAGE)),
        WireError::MissingAzureKey => failure("Exception", String::from(AZURE_KEY_MESSAGE)),
        WireError::Vertex => failure("Exception", String::from(VERTEX_MESSAGE)),
        WireError::Transport { .. } => failure("APIConnectionError", text),
        WireError::Decode { .. } => failure("APIResponseValidationError", text),
        WireError::JobId(_) => return Raised::Python(Box::new(PyValueError::new_err(text))),
        WireError::Body { .. }
        | WireError::Encode { .. }
        | WireError::Url { .. }
        | WireError::Header { .. }
        | WireError::Proxy { .. }
        | WireError::Config { .. }
        | WireError::Client { .. } => failure("Exception", text),
    };
    Raised::Failure(Box::new(payload))
}

// The error a call's own steps raised, before it becomes the class LiteLLM shows the caller.
pub(crate) enum TuningFailure {
    Create(FineTuningCreateError<WireError>),
    Read(FineTuningReadError<WireError>),
    Cancel(FineTuningCancelError<WireError>),
}

// One place maps every fine-tuning failure, so the create, read and cancel paths cannot drift apart.
impl From<TuningFailure> for Raised {
    fn from(outcome: TuningFailure) -> Self {
        let message = match &outcome {
            TuningFailure::Create(error) => error.to_string(),
            TuningFailure::Read(error) => error.to_string(),
            TuningFailure::Cancel(error) => error.to_string(),
        };
        match outcome {
            TuningFailure::Create(error) => match error {
                FineTuningCreateError::Unsupported { provider } => {
                    Self::Failure(Box::new(unsupported_failure(&provider, message)))
                }
                FineTuningCreateError::Timeout(_) | FineTuningCreateError::Data(_) => {
                    Self::Python(Box::new(PyValueError::new_err(message)))
                }
                FineTuningCreateError::Dispatch(source) => wire_failure(source),
            },
            TuningFailure::Read(error) => match error {
                FineTuningReadError::Timeout(_) => {
                    Self::Python(Box::new(PyValueError::new_err(message)))
                }
                FineTuningReadError::UnsupportedList { provider }
                | FineTuningReadError::UnsupportedRetrieve { provider } => {
                    Self::Failure(Box::new(unsupported_failure(&provider, message)))
                }
                FineTuningReadError::Decode(_) | FineTuningReadError::Page { .. } => {
                    Self::Failure(Box::new(failure("APIResponseValidationError", message)))
                }
                FineTuningReadError::Dispatch(source) => wire_failure(source),
            },
            TuningFailure::Cancel(error) => match error {
                FineTuningCancelError::Timeout(_) => {
                    Self::Python(Box::new(PyValueError::new_err(message)))
                }
                FineTuningCancelError::Unsupported { provider } => {
                    Self::Failure(Box::new(unsupported_failure(&provider, message)))
                }
                FineTuningCancelError::Dispatch(source) => wire_failure(source),
            },
        }
    }
}
