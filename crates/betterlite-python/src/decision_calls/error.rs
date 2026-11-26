use std::time::Duration;

use betterlite_config::tls::TlsPolicyError;
use betterlite_core::decisions::DecisionsEntryError;
use betterlite_env::EnvError;
use betterlite_models::sandbox_dispatch::SandboxToolEntryError;
use betterlite_provider::e2b::E2bError;
use betterlite_provider::opensandbox::helpers::OpenSandboxError;
use betterlite_provider::opensandbox::lifecycle::LifecycleError;
use betterlite_provider::sandbox_dispatch::error::SandboxLifecycleError;
use betterlite_provider::sandbox_dispatch::registry::SandboxRegistryError;
use betterlite_redact::error::RedactionError;
use betterlite_sdk::call_wrapper::error::plain_error;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::types::PyType;
use pyo3::{Py, PyErr, Python};

use super::convert::{ConvertError, NativeDecisionPayload, TIMEOUT_ERROR, VALUE_ERROR};
use crate::completion::error::SessionError as CompletionSessionError;

const E2B_MISSING_KEY: &str = "E2B API key not set. Set E2B_API_KEY or pass api_key=...";
const E2B_BARE_ID: &str = "Cannot run code from a sandbox id alone. e2b secure sandboxes require the access token returned by acreate_sandbox; pass the ContainerHandle it returned instead of a bare sandbox id.";

#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error(transparent)]
    Json(#[from] ConvertError),
    #[error(transparent)]
    Completion(#[from] CompletionSessionError),
    #[error("the proxy environment could not be read: {source}")]
    Proxy {
        #[source]
        source: EnvError,
    },
    #[error("the TLS policy could not be resolved: {source}")]
    Tls {
        #[source]
        source: TlsPolicyError,
    },
    #[error("the redactor could not be built: {source}")]
    Redactor {
        #[source]
        source: RedactionError,
    },
}

#[derive(Debug)]
pub(crate) enum SessionFailure {
    Raised(Box<NativeDecisionPayload>),
    Invalid(ConvertError),
    Setup(SessionError),
}

impl SessionFailure {
    pub(crate) fn into_pyerr(self, py: Python<'_>, error_type: &Py<PyType>) -> PyErr {
        match self {
            Self::Raised(payload) => native_error(py, error_type, payload.as_ref()),
            Self::Invalid(error) => PyErr::from(error),
            Self::Setup(error) => PyErr::from(error),
        }
    }
}

pub(crate) fn native_error(
    py: Python<'_>,
    error_type: &Py<PyType>,
    payload: &NativeDecisionPayload,
) -> PyErr {
    let message = payload.message.clone();
    match serde_json::to_string(payload) {
        Ok(text) => PyErr::from_type(error_type.bind(py).clone(), text),
        Err(_encoding) => PyRuntimeError::new_err(message),
    }
}

impl From<DecisionsEntryError> for SessionFailure {
    fn from(failure: DecisionsEntryError) -> Self {
        Self::Raised(Box::new(NativeDecisionPayload::from_class(
            &failure.class(),
        )))
    }
}

impl From<SandboxRegistryError> for SessionFailure {
    fn from(failure: SandboxRegistryError) -> Self {
        Self::Raised(Box::new(NativeDecisionPayload::from_class(&plain_error(
            &failure.to_string(),
        ))))
    }
}

// LiteLLM raises builtins for the sandbox's own misuse, whose names the exception table cannot build
impl From<SandboxToolEntryError> for SessionFailure {
    fn from(failure: SandboxToolEntryError) -> Self {
        Self::Raised(Box::new(NativeDecisionPayload::builtin(
            VALUE_ERROR,
            failure.to_string(),
        )))
    }
}

impl From<SandboxLifecycleError> for SessionFailure {
    fn from(failure: SandboxLifecycleError) -> Self {
        if let Some((kind, message)) = litellm_builtin(&failure) {
            return Self::Raised(Box::new(NativeDecisionPayload::builtin(kind, message)));
        }
        Self::Raised(Box::new(NativeDecisionPayload::from_class(&plain_error(
            &failure.to_string(),
        ))))
    }
}

fn litellm_builtin(error: &SandboxLifecycleError) -> Option<(&'static str, String)> {
    match error {
        SandboxLifecycleError::UnsupportedProvider { provider } => Some((
            VALUE_ERROR,
            format!("Code execution is not supported for provider: {provider}"),
        )),
        SandboxLifecycleError::ContainerHandleRequired { .. } => {
            Some((VALUE_ERROR, String::from(E2B_BARE_ID)))
        }
        SandboxLifecycleError::E2b(e2b_error) => {
            e2b_message(e2b_error).map(|message| (VALUE_ERROR, String::from(message)))
        }
        SandboxLifecycleError::OpenSandbox(LifecycleError::Sandbox(sandbox_error)) => {
            opensandbox_builtin(sandbox_error)
        }
        SandboxLifecycleError::Logging(_) | SandboxLifecycleError::OpenSandbox(_) => None,
    }
}

const fn e2b_message(failure: &E2bError) -> Option<&'static str> {
    match failure {
        E2bError::MissingApiKey => Some(E2B_MISSING_KEY),
        E2bError::MissingAccessToken => Some(E2B_BARE_ID),
        E2bError::HttpStatus { .. }
        | E2bError::Url { .. }
        | E2bError::InvalidHeader { .. }
        | E2bError::Encode { .. }
        | E2bError::CreateResponse { .. }
        | E2bError::ResponseTooLarge { .. }
        | E2bError::OutputTooLarge { .. }
        | E2bError::OutputNotUtf8 { .. }
        | E2bError::Transport { .. }
        | E2bError::Body { .. } => None,
    }
}

fn opensandbox_builtin(failure: &OpenSandboxError) -> Option<(&'static str, String)> {
    if let OpenSandboxError::MissingApiBase { variable } = failure {
        return Some((
            VALUE_ERROR,
            format!("OpenSandbox api_base is required. Pass api_base or set {variable}."),
        ));
    }
    if let OpenSandboxError::NotRunning {
        sandbox_id,
        timeout,
    } = failure
    {
        return Some((
            TIMEOUT_ERROR,
            format!(
                "OpenSandbox sandbox {sandbox_id} was not Running within {} seconds",
                seconds(*timeout)
            ),
        ));
    }
    if let OpenSandboxError::EndpointNotReady {
        sandbox_id,
        timeout,
        ..
    } = failure
    {
        return Some((
            TIMEOUT_ERROR,
            format!(
                "OpenSandbox execd endpoint for {sandbox_id} was not ready within {} seconds",
                seconds(*timeout)
            ),
        ));
    }
    None
}

// Python's str(float) prints 30.0 where Rust's Display prints 30
fn seconds(timeout: Duration) -> String {
    let mut text = timeout.as_secs_f64().to_string();
    if !text.contains('.') {
        text.push_str(".0");
    }
    text
}

impl From<SessionError> for SessionFailure {
    fn from(error: SessionError) -> Self {
        Self::Setup(error)
    }
}

impl From<ConvertError> for SessionFailure {
    fn from(error: ConvertError) -> Self {
        Self::Invalid(error)
    }
}

impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        let message = error.to_string();
        if matches!(&error, ConvertError::Json { .. }) {
            return PyValueError::new_err(message);
        }
        PyRuntimeError::new_err(message)
    }
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        if let SessionError::Json(inner) = &error {
            return PyValueError::new_err(inner.to_string());
        }
        PyRuntimeError::new_err(error.to_string())
    }
}
