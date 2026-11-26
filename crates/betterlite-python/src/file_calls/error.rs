use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::endpoints::initialize::EndpointError;
use betterlite_core::api::file_provider_read::error::{
    FileReadError, UNSUPPORTED_BODY, UNSUPPORTED_MODEL,
};
use betterlite_core::api::file_provider_write::{BadRequest, FileWriteError};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::files::error::{FilesError, StatusError};
use indexmap::IndexMap;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{
    PyException, PyKeyError, PyNotImplementedError, PyRuntimeError, PyValueError,
};
use serde::Serialize;

use super::dispatch::DispatchError;

create_exception!(betterlite_native, NativeFileCallError, PyException);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct FileFailurePayload {
    #[serde(rename = "type")]
    pub(crate) class: String,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) llm_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) body: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Failure {
    Value(String),
    Runtime(String),
    Unsupported(String),
    Key(String),
    Payload(Box<FileFailurePayload>),
}

impl From<Failure> for PyErr {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Payload(payload) => encoded_error(&payload),
            Failure::Value(message) => PyValueError::new_err(message),
            Failure::Runtime(message) => PyRuntimeError::new_err(message),
            Failure::Unsupported(message) => PyNotImplementedError::new_err(message),
            Failure::Key(message) => PyKeyError::new_err(message),
        }
    }
}

fn encoded_error(payload: &FileFailurePayload) -> PyErr {
    match serde_json::to_string(payload) {
        Ok(text) => NativeFileCallError::new_err(text),
        Err(source) => PyRuntimeError::new_err(source.to_string()),
    }
}

pub(crate) fn write_failure(error: &FileWriteError<DispatchError>) -> Failure {
    if let FileWriteError::Provider(source) = error {
        return dispatch_failure(source);
    }
    if let Some(rejected) = error.bad_request() {
        return rejected_write(error, &rejected);
    }
    Failure::Value(error.to_string())
}

pub(crate) fn read_failure(error: &FileReadError<DispatchError>) -> Failure {
    match error {
        FileReadError::Dispatch(source) => return dispatch_failure(source),
        FileReadError::Unsupported { provider, .. } => {
            return Failure::Payload(Box::new(unsupported_read(error, provider)));
        }
        FileReadError::Timeout(_) | FileReadError::UnknownProvider { .. } => {}
    }
    Failure::Value(error.to_string())
}

fn dispatch_failure(error: &DispatchError) -> Failure {
    match error {
        DispatchError::Files(source) => files_failure(source),
        DispatchError::Azure(source) => azure_failure(source),
        DispatchError::Unbuilt { config } => unbuilt_failure(config),
        DispatchError::Reply => Failure::Runtime(error.to_string()),
    }
}

fn unbuilt_failure(config: &str) -> Failure {
    Failure::Unsupported(format!(
        "the {config} file call is not implemented in this build"
    ))
}

fn rejected_write(error: &FileWriteError<DispatchError>, rejected: &BadRequest<'_>) -> Failure {
    Failure::Payload(Box::new(FileFailurePayload {
        class: String::from("BadRequestError"),
        message: error.to_string(),
        status_code: Some(400),
        llm_provider: Some(rejected.llm_provider.to_owned()),
        model: Some(String::from(rejected.model)),
        body: Some(JsonValue::Text(rejected.body.to_owned())),
    }))
}

// file_retrieve and file_list raise the same 400 with their own function name in the message
fn unsupported_read(error: &FileReadError<DispatchError>, provider: &str) -> FileFailurePayload {
    FileFailurePayload {
        class: String::from("BadRequestError"),
        message: error.to_string(),
        status_code: Some(400),
        llm_provider: Some(provider.to_owned()),
        model: Some(String::from(UNSUPPORTED_MODEL)),
        body: Some(JsonValue::Text(String::from(UNSUPPORTED_BODY))),
    }
}

pub(crate) fn files_failure(error: &FilesError) -> Failure {
    if let FilesError::Status(status) = error {
        return Failure::Payload(Box::new(status_payload(status)));
    }
    if let FilesError::Transport { .. } = error {
        return payload("APIConnectionError", error.to_string(), Some(500), None);
    }
    if let FilesError::Decode { .. } | FilesError::Body { .. } = error {
        return payload("Exception", error.to_string(), None, None);
    }
    Failure::Value(error.to_string())
}

fn payload(
    class: &str,
    message: String,
    status_code: Option<u16>,
    body: Option<JsonValue>,
) -> Failure {
    Failure::Payload(Box::new(FileFailurePayload {
        class: String::from(class),
        message,
        status_code,
        llm_provider: None,
        model: None,
        body,
    }))
}

fn status_payload(status: &StatusError) -> FileFailurePayload {
    FileFailurePayload {
        class: String::from(class_for_status(status.status)),
        message: status.message.clone(),
        status_code: Some(status.status),
        llm_provider: None,
        model: None,
        body: status_body(status),
    }
}

fn status_body(status: &StatusError) -> Option<JsonValue> {
    if status.error_type.is_none() && status.param.is_none() && status.code.is_none() {
        return None;
    }
    let mut fields = IndexMap::new();
    if let Some(value) = &status.error_type {
        fields.insert(String::from("type"), JsonValue::Text(value.clone()));
    }
    if let Some(value) = &status.param {
        fields.insert(String::from("param"), JsonValue::Text(value.clone()));
    }
    if let Some(value) = &status.code {
        fields.insert(String::from("code"), JsonValue::Text(value.clone()));
    }
    Some(JsonValue::Object(fields))
}

const fn class_for_status(status: u16) -> &'static str {
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

fn azure_endpoint_failure(source: &EndpointError) -> Failure {
    match source.class() {
        "KeyError" => Failure::Key(source.to_string()),
        _ => Failure::Value(source.to_string()),
    }
}

pub(crate) fn azure_failure(error: &AzureFilesError) -> Failure {
    if let AzureFilesError::Files { source } = error {
        return files_failure(source);
    }
    if let AzureFilesError::Endpoint { source } = error {
        return azure_endpoint_failure(source);
    }
    if let AzureFilesError::TokenProvider { .. } = error {
        return Failure::Runtime(error.to_string());
    }
    Failure::Value(error.to_string())
}
