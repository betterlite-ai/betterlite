use std::str::Utf8Error;

use betterlite_models::decisions::Json;
use serde::de::DeserializeOwned;
use thiserror::Error;

use super::transport::{HttpResponse, RequestFailure};

#[derive(Debug, Error)]
#[error("status_code: {status_code}, body: {}", body_text(body))]
pub struct LangfusePromptError {
    pub status_code: u16,
    pub body: Json,
}

#[derive(Debug, Error)]
pub enum LangfuseRestError {
    #[error(transparent)]
    Api(#[from] LangfusePromptError),
    #[error(transparent)]
    Request(#[from] RequestFailure),
    #[error("the Langfuse response is not the declared JSON shape")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Langfuse response body is not UTF-8")]
    Body {
        status: u16,
        #[source]
        source: Utf8Error,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthCheckFailure {
    pub reason: String,
}

pub fn auth_check_failure(reason: String) -> AuthCheckFailure {
    tracing::warn!("Langfuse auth check failed: {reason}");
    AuthCheckFailure { reason }
}

pub fn decode_response<T>(response: &HttpResponse) -> Result<T, LangfuseRestError>
where
    T: DeserializeOwned,
{
    if !(200..300).contains(&response.status) {
        return Err(api_failure(response.status, response));
    }
    match serde_json::from_slice(&response.body) {
        Ok(value) => Ok(value),
        Err(source) if source.is_data() => Err(LangfuseRestError::Decode { source }),
        Err(_) => Err(api_failure(response.status, response)),
    }
}

fn api_failure(status: u16, response: &HttpResponse) -> LangfuseRestError {
    match response.json() {
        Ok(body) => LangfuseRestError::Api(LangfusePromptError {
            status_code: status,
            body,
        }),
        Err(source) => LangfuseRestError::Body { status, source },
    }
}

fn body_text(body: &Json) -> String {
    match body {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(body).unwrap_or_default()
        }
    }
}
