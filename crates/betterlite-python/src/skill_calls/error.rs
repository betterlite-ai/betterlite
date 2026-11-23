use std::collections::BTreeMap;

use betterlite_core::api::skills::get_delete::call::LITELLM_PROXY_PROVIDER;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
use serde::Serialize;

use crate::skill_calls::convert::RequestError;

create_exception!(betterlite_native, NativeSkillError, PyException);

// litellm_proxy/skills/handler.py's _get_prisma_client, which the Python package never initializes
pub(crate) const NO_DATABASE: &str =
    "Prisma client is not initialized. Database connection required for LiteLLM skills.";

// One failed skill call: the class Python rebuilds into the matching LiteLLM exception.
#[derive(Debug)]
pub(crate) struct SkillFailure {
    pub(crate) class: ErrorClass,
}

impl From<RequestError> for SkillFailure {
    fn from(error: RequestError) -> Self {
        Self {
            class: error.class(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error("the settings are not a JSON object: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the skill session could not be built: {message}")]
    Setup { message: String },
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        match &error {
            SessionError::Json { .. } => PyValueError::new_err(error.to_string()),
            SessionError::Setup { .. } => PyRuntimeError::new_err(error.to_string()),
        }
    }
}

// The JSON text a NativeSkillError carries: LiteLLM's exception name and the fields its class keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NativeErrorPayload {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) llm_provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) num_retries: Option<i64>,
    pub(crate) max_retries: Option<i64>,
}

impl NativeErrorPayload {
    pub(crate) fn from_class(class: &ErrorClass) -> Self {
        let metadata = class.metadata();
        let mut headers = BTreeMap::new();
        if let ErrorClass::RateLimit(details) = class {
            headers.clone_from(&details.headers);
        }
        Self {
            kind: class_name(class).to_owned(),
            message: class_message(class),
            status_code: class_status(class),
            llm_provider: metadata.provider,
            model: metadata.model,
            headers,
            num_retries: None,
            max_retries: metadata.retry.max_retries.map(i64::from),
        }
    }
}

pub(crate) fn native_error(failure: &SkillFailure) -> PyErr {
    let payload = NativeErrorPayload::from_class(&failure.class);
    match serde_json::to_string(&payload) {
        Ok(text) => NativeSkillError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(class_message(&failure.class)),
    }
}

// LiteLLM reaches an uninitialized Prisma client, and exception_type maps its ValueError to a connection error.
pub(crate) fn no_database_failure(redactor: &Redactor) -> SkillFailure {
    let raw = RawException {
        model: String::new(),
        custom_llm_provider: LITELLM_PROXY_PROVIDER.to_owned(),
        error_str: NO_DATABASE.to_owned(),
        status_code: None,
        status_code_is_synthesized: false,
        message: Some(NO_DATABASE.to_owned()),
        body: None,
        exception_type: "ValueError".to_owned(),
    };
    SkillFailure {
        class: dispatch(&DispatchInput::Raw(Box::new(raw)), redactor),
    }
}
