use std::str::Utf8Error;

use betterlite_models::decisions::Json;
use thiserror::Error;

use super::transport::RequestFailure;

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
        #[from]
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

fn body_text(body: &Json) -> String {
    match body {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(body).unwrap_or_default()
        }
    }
}
