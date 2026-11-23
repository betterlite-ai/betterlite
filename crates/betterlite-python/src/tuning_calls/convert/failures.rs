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

pub(crate) fn provider_error(response: UpstreamResponse) -> WireError {
    let headers = response_headers(&response.headers);
    let body = match serde_json::from_slice::<Json>(&response.body) {
        Ok(value) => value,
        Err(_source) => Json::Text(
            from_utf8(&response.body)
                .map(str::to_owned)
                .unwrap_or_default(),
        ),
    };
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

fn exception_failure(message: String) -> Failure {
    failure("Exception", message)
}

fn connection_failure(message: String) -> Failure {
    failure("APIConnectionError", message)
}

fn validation_failure(message: String) -> Failure {
    failure("APIResponseValidationError", message)
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
    Failure {
        kind: "BadRequestError".to_owned(),
        message,
        status_code: Some(400),
        llm_provider: Some(provider.to_owned()),
        model: Some("n/a".to_owned()),
        body: Some(Json::Text(UNSUPPORTED_BODY.to_owned())),
        headers: BTreeMap::new(),
    }
}

pub(crate) fn wire_failure(source: WireError) -> Raised {
    let message = source.to_string();
    match source {
        WireError::Provider {
            status,
            body,
            headers,
        } => Raised::Failure(Box::new(provider_failure(status, body, headers))),
        WireError::Transport { .. } => Raised::Failure(Box::new(connection_failure(message))),
        WireError::Decode { .. } => Raised::Failure(Box::new(validation_failure(message))),
        WireError::MissingOpenAiKey => Raised::Failure(Box::new(exception_failure(String::from(
            OPENAI_KEY_MESSAGE,
        )))),
        WireError::MissingAzureBase => {
            Raised::Python(PyValueError::new_err(String::from(AZURE_BASE_MESSAGE)))
        }
        WireError::MissingAzureKey => {
            Raised::Failure(Box::new(exception_failure(String::from(AZURE_KEY_MESSAGE))))
        }
        WireError::Vertex => {
            Raised::Failure(Box::new(exception_failure(String::from(VERTEX_MESSAGE))))
        }
        WireError::Json { .. }
        | WireError::Body { .. }
        | WireError::Encode { .. }
        | WireError::Url { .. }
        | WireError::Header { .. }
        | WireError::Proxy { .. }
        | WireError::Config { .. }
        | WireError::Client { .. } => Raised::Failure(Box::new(exception_failure(message))),
    }
}

pub(crate) fn create_failure(error: FineTuningCreateError<WireError>) -> Raised {
    let message = error.to_string();
    match error {
        FineTuningCreateError::Unsupported { provider } => {
            Raised::Failure(Box::new(unsupported_failure(&provider, message)))
        }
        FineTuningCreateError::Timeout(_) | FineTuningCreateError::Data(_) => {
            Raised::Python(PyValueError::new_err(message))
        }
        FineTuningCreateError::Dispatch(source) => wire_failure(source),
    }
}

pub(crate) fn read_failure(error: FineTuningReadError<WireError>) -> Raised {
    let message = error.to_string();
    match error {
        FineTuningReadError::Timeout(_) => Raised::Python(PyValueError::new_err(message)),
        FineTuningReadError::UnsupportedList { provider }
        | FineTuningReadError::UnsupportedRetrieve { provider } => {
            Raised::Failure(Box::new(unsupported_failure(&provider, message)))
        }
        FineTuningReadError::Decode(_) | FineTuningReadError::Page { .. } => {
            Raised::Failure(Box::new(validation_failure(message)))
        }
        FineTuningReadError::Dispatch(source) => wire_failure(source),
    }
}

pub(crate) fn cancel_failure(error: FineTuningCancelError<WireError>) -> Raised {
    let message = error.to_string();
    match error {
        FineTuningCancelError::Timeout(_) => Raised::Python(PyValueError::new_err(message)),
        FineTuningCancelError::Unsupported { provider } => {
            Raised::Failure(Box::new(unsupported_failure(&provider, message)))
        }
        FineTuningCancelError::Dispatch(source) => wire_failure(source),
    }
}
