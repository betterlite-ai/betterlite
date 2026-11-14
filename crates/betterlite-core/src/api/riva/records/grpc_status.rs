use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// grpc.StatusCode.name -> the HTTP-equivalent code litellm's error classifiers branch on
pub const GRPC_HTTP_STATUS: [(&str, u16); 17] = [
    ("OK", 200),
    ("CANCELLED", 499),
    ("UNKNOWN", 500),
    ("INVALID_ARGUMENT", 400),
    ("DEADLINE_EXCEEDED", 504),
    ("NOT_FOUND", 404),
    ("ALREADY_EXISTS", 409),
    ("PERMISSION_DENIED", 403),
    ("RESOURCE_EXHAUSTED", 429),
    ("FAILED_PRECONDITION", 400),
    ("ABORTED", 409),
    ("OUT_OF_RANGE", 400),
    ("UNIMPLEMENTED", 501),
    ("INTERNAL", 500),
    ("UNAVAILABLE", 503),
    ("DATA_LOSS", 500),
    ("UNAUTHENTICATED", 401),
];

pub const DEFAULT_GRPC_HTTP_STATUS: u16 = 500;

// What reading the SDK error's code attribute found: it may be absent, not callable, raise, or return a status code
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CodeAttribute {
    Missing,
    NotCallable,
    Raises,
    Returns { name: Json },
}

// What reading the SDK error's details attribute found, with the same four outcomes
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DetailsAttribute {
    Missing,
    NotCallable,
    Raises,
    Returns { value: Json },
}

// The bridge reads these three observations off a Python SDK error; nothing else of the exception is needed
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GrpcErrorObservation {
    pub code: CodeAttribute,
    pub details: DetailsAttribute,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct RivaException {
    pub status_code: u16,
    pub message: String,
}

// only a callable code whose name is a string names a status; a nonempty details string wins over str(error)
fn describe(observation: &GrpcErrorObservation) -> (Option<&str>, &str) {
    let name = match &observation.code {
        CodeAttribute::Returns {
            name: Json::Text(name),
        } => Some(name.as_str()),
        CodeAttribute::Missing
        | CodeAttribute::NotCallable
        | CodeAttribute::Raises
        | CodeAttribute::Returns { .. } => None,
    };
    let detail = match &observation.details {
        DetailsAttribute::Returns {
            value: Json::Text(text),
        } if !text.is_empty() => text.as_str(),
        DetailsAttribute::Missing
        | DetailsAttribute::NotCallable
        | DetailsAttribute::Raises
        | DetailsAttribute::Returns { .. } => observation.text.as_str(),
    };
    (name, detail)
}

fn http_status(name: &str) -> Option<u16> {
    let mut status = None;
    for (candidate, code) in GRPC_HTTP_STATUS {
        if candidate == name {
            status = Some(code);
        }
    }
    status
}

#[must_use]
pub fn grpc_error_to_exception(observation: &GrpcErrorObservation) -> RivaException {
    let (name, detail) = describe(observation);
    let name = name.filter(|name| !name.is_empty());
    let status_code = name
        .and_then(http_status)
        .unwrap_or(DEFAULT_GRPC_HTTP_STATUS);
    let message = match name {
        Some(name) => format!("NVIDIA Riva gRPC error ({name}): {detail}"),
        None => format!("NVIDIA Riva error: {detail}"),
    };
    RivaException {
        status_code,
        message,
    }
}
