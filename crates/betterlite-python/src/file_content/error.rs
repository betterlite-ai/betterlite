use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::endpoints::initialize::EndpointError;
use betterlite_core::api::file_content_stream::error::FileContentError;
use betterlite_core::api::file_stream_logging::source::StreamError;
use betterlite_error::error::class_message;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::files::error::{FilesError, StatusError};
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use indexmap::IndexMap;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{
    PyException, PyKeyError, PyNotImplementedError, PyRuntimeError, PyTypeError, PyValueError,
};
use serde::Serialize;

use super::dispatch::DispatchError;

create_exception!(betterlite_native, NativeFileContentError, PyException);

// What _completion/errors.py builds a LiteLLM exception from: its class, its message and its attributes.
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

// The Python error one failure becomes: a builtin class, or an exception named by its payload.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Failure {
    Value(String),
    Type(String),
    Runtime(String),
    Unsupported(String),
    Key(String),
    Payload(Box<FileFailurePayload>),
}

pub(crate) fn native_error(failure: Failure) -> PyErr {
    match failure {
        Failure::Value(message) => PyValueError::new_err(message),
        Failure::Type(message) => PyTypeError::new_err(message),
        Failure::Runtime(message) => PyRuntimeError::new_err(message),
        Failure::Unsupported(message) => PyNotImplementedError::new_err(message),
        Failure::Key(message) => PyKeyError::new_err(message),
        Failure::Payload(payload) => match serde_json::to_string(&payload) {
            Ok(text) => NativeFileContentError::new_err(text),
            Err(source) => PyRuntimeError::new_err(source.to_string()),
        },
    }
}

pub(crate) fn failure_for(error: &FileContentError<DispatchError>) -> Failure {
    match error {
        FileContentError::Timeout(_) | FileContentError::UnknownProvider { .. } => {
            Failure::Value(error.to_string())
        }
        FileContentError::Unsupported { .. }
        | FileContentError::UnsupportedStreaming { .. }
        | FileContentError::SyncVertexStreaming => rejected_provider(error),
        FileContentError::Reply => Failure::Runtime(error.to_string()),
        FileContentError::Dispatch(source) => dispatch_failure(source),
    }
}

// the placeholder 400 LiteLLM attaches to every provider it refuses
fn rejected_provider(error: &FileContentError<DispatchError>) -> Failure {
    let Some(rejected) = error.bad_request() else {
        return Failure::Runtime(error.to_string());
    };
    Failure::Payload(Box::new(FileFailurePayload {
        class: String::from("BadRequestError"),
        message: error.to_string(),
        status_code: Some(400),
        llm_provider: Some(rejected.llm_provider.to_owned()),
        model: Some(String::from(rejected.model)),
        body: Some(JsonValue::Text(rejected.body.to_owned())),
    }))
}

fn dispatch_failure(error: &DispatchError) -> Failure {
    match error {
        DispatchError::Files(source) => files_failure(source),
        DispatchError::Azure(source) => azure_failure(source),
        DispatchError::Reply => Failure::Runtime(error.to_string()),
        DispatchError::Unbuilt { config } => Failure::Unsupported(format!(
            "the {config} file content call is not implemented in this build"
        )),
    }
}

pub(crate) fn files_failure(error: &FilesError) -> Failure {
    match error {
        FilesError::Status(status) => Failure::Payload(Box::new(status_payload(status))),
        FilesError::Transport { .. } => {
            payload("APIConnectionError", error.to_string(), None, None)
        }
        FilesError::Decode { .. } | FilesError::Body { .. } => {
            payload("Exception", error.to_string(), None, None)
        }
        FilesError::NotInitialized
        | FilesError::NotAsync
        | FilesError::EmptyValue { .. }
        | FilesError::DotSegment { .. }
        | FilesError::BaseUrl { .. }
        | FilesError::Header { .. } => Failure::Value(error.to_string()),
    }
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

pub(crate) fn azure_failure(error: &AzureFilesError) -> Failure {
    match error {
        AzureFilesError::Files { source } => files_failure(source),
        AzureFilesError::Endpoint { source } => endpoint_failure(source),
        AzureFilesError::TokenProvider { .. } => Failure::Runtime(error.to_string()),
        AzureFilesError::ClientMissing
        | AzureFilesError::NotAsyncClient
        | AzureFilesError::MissingCredentials
        | AzureFilesError::UnableToHandleAuth
        | AzureFilesError::TokenNotString { .. }
        | AzureFilesError::MissingBaseUrl
        | AzureFilesError::BaseUrl { .. }
        | AzureFilesError::Header { .. }
        | AzureFilesError::Timeout { .. } => Failure::Value(error.to_string()),
    }
}

// the azure endpoints layer names the class a resolved endpoint raises
fn endpoint_failure(error: &EndpointError) -> Failure {
    match error.class() {
        "KeyError" => Failure::Key(error.to_string()),
        _ => Failure::Value(error.to_string()),
    }
}

pub(crate) fn stream_failure(error: &StreamError) -> Failure {
    match error {
        StreamError::UnsupportedSync | StreamError::UnsupportedAsync => {
            Failure::Type(error.to_string())
        }
        StreamError::Source { failure } | StreamError::Cleanup { failure } => payload(
            class_name(failure),
            class_message(failure),
            class_status(failure),
            None,
        ),
        StreamError::Cancelled
        | StreamError::Callback { .. }
        | StreamError::CleanupAborted { .. } => Failure::Runtime(error.to_string()),
    }
}
