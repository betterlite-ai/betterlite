use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::team_fields::Settings as TeamFieldsSettings;
use betterlite_config::sections::team_fields::ValidationTimeout;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_pyhost::blocking_work::{WorkError, WorkTracker};
use betterlite_pyhost::call_error::{CallError, PythonError};
use betterlite_pyhost::callable_handle::CallableHandle;
use betterlite_pyhost::host::PythonHost;
use betterlite_pyhost::json_values::{from_object, to_object};
use pyo3::exceptions::PyValueError;
use pyo3::types::{PyAnyMethods as _, PyDict, PyDictMethods as _};
use pyo3::{Bound, IntoPyObjectExt as _, PyAny, PyResult, Python};

use crate::python_hooks::slots::LoadedHook;
use crate::team_request_contracts::metadata::TeamMetadataFieldSchema;

use super::metadata::{
    DEFAULT_REJECTED_MESSAGE, MetadataMap, MetadataRequest, TeamMetadataValidationPayload,
    TeamMetadataValidationResult, build_payload,
};

pub const NOT_ASYNC_MESSAGE: &str = "custom_team_metadata_validate must be an async function";

type PythonObject<'py> = Bound<'py, PyAny>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TeamMetadataError {
    #[error("{}", NOT_ASYNC_MESSAGE)]
    NotAsync,
    #[error("{message}")]
    Unavailable { message: String },
    #[error("{message}")]
    Rejected { message: String },
}

impl TeamMetadataError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::NotAsync => 500,
            Self::Unavailable { .. } => 503,
            Self::Rejected { .. } => 400,
        }
    }

    #[must_use]
    pub const fn message(&self) -> &str {
        match self {
            Self::NotAsync => NOT_ASYNC_MESSAGE,
            Self::Unavailable { message } | Self::Rejected { message } => message.as_str(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ValidatorFailure {
    #[error("the metadata validator did not finish within {after:?}")]
    TimedOut { after: Duration },
    #[error("the metadata validator raised an error")]
    Raised {
        #[source]
        source: CallError,
    },
    #[error("the metadata validator returned a value with no JSON form")]
    NotJson,
    #[error("the metadata validator could not be prepared")]
    Python(#[source] PythonError),
}

impl From<CallError> for ValidatorFailure {
    fn from(source: CallError) -> Self {
        match source {
            CallError::TimedOut { after } => Self::TimedOut { after },
            CallError::Work(WorkError::TimedOut { after }) => Self::TimedOut { after },
            CallError::Python(source) => Self::Raised {
                source: CallError::Python(source),
            },
            CallError::Value(source) => Self::Raised {
                source: CallError::Value(source),
            },
            CallError::Work(
                work @ (WorkError::ShuttingDown
                | WorkError::Closed { .. }
                | WorkError::Panicked(_)
                | WorkError::Cancelled),
            ) => Self::Raised {
                source: CallError::Work(work),
            },
        }
    }
}

#[async_trait]
pub trait TeamMetadataValidator: Send + Sync {
    fn is_async(&self) -> bool;

    async fn invoke(
        &self,
        payload: &TeamMetadataValidationPayload,
        timeout: ValidationTimeout,
    ) -> Result<JsonData, ValidatorFailure>;
}

pub struct PythonValidator {
    handle: CallableHandle,
    host: PythonHost,
    tracker: Arc<WorkTracker>,
}

impl PythonValidator {
    pub fn load(hook: LoadedHook) -> Result<Self, CallError> {
        let host = hook.host;
        let tracker = hook.tracker;
        let handle = hook.object.loaded.handle(&host)?;
        Ok(Self {
            handle,
            host,
            tracker,
        })
    }
}

#[async_trait]
impl TeamMetadataValidator for PythonValidator {
    fn is_async(&self) -> bool {
        self.handle.is_async
    }

    async fn invoke(
        &self,
        payload: &TeamMetadataValidationPayload,
        timeout: ValidationTimeout,
    ) -> Result<JsonData, ValidatorFailure> {
        let argument = self
            .host
            .attach(|py| {
                payload_object(py, payload)
                    .map(Bound::unbind)
                    .map_err(|error| PythonError::from_pyerr(py, &error))
            })
            .map_err(ValidatorFailure::Python)?;
        let result = self
            .handle
            .call_raw(
                &self.host,
                &self.tracker,
                deadline(timeout),
                vec![argument],
                BTreeMap::new(),
            )
            .await
            .map_err(ValidatorFailure::from)?;
        let value = self
            .host
            .attach(|py| from_object(result.bind(py)))
            .map_err(|_error| ValidatorFailure::NotJson)?;
        serde_json::from_value(value).map_err(|_error| ValidatorFailure::NotJson)
    }
}

fn deadline(timeout: ValidationTimeout) -> Duration {
    match timeout {
        ValidationTimeout::Finite(duration) => duration,
        ValidationTimeout::Unlimited => Duration::MAX,
    }
}

// LiteLLM hands the callback its pydantic payload, so the namespace carries the same attribute names.
fn payload_object<'py>(
    py: Python<'py>,
    payload: &TeamMetadataValidationPayload,
) -> PyResult<PythonObject<'py>> {
    let requester = simple_namespace(
        py,
        vec![
            (
                "user_id",
                optional_text(py, payload.requester.user_id.as_deref())?,
            ),
            (
                "user_email",
                optional_text(py, payload.requester.user_email.as_deref())?,
            ),
            (
                "user_role",
                optional_text(py, payload.requester.user_role.as_deref())?,
            ),
        ],
    )?;
    simple_namespace(
        py,
        vec![
            (
                "operation",
                payload.operation.as_str().into_bound_py_any(py)?,
            ),
            ("metadata", json_tree(py, &payload.metadata)?),
            (
                "existing_metadata",
                optional_metadata(py, payload.existing_metadata.as_ref())?,
            ),
            ("team_id", optional_text(py, payload.team_id.as_deref())?),
            (
                "team_alias",
                optional_text(py, payload.team_alias.as_deref())?,
            ),
            ("requester", requester),
        ],
    )
}

fn simple_namespace<'py>(
    py: Python<'py>,
    entries: Vec<(&'static str, PythonObject<'py>)>,
) -> PyResult<PythonObject<'py>> {
    let keywords = PyDict::new(py);
    for (name, value) in entries {
        keywords.set_item(name, value)?;
    }
    py.import("types")?
        .getattr("SimpleNamespace")?
        .call((), Some(&keywords))
}

fn optional_text<'py>(py: Python<'py>, value: Option<&str>) -> PyResult<PythonObject<'py>> {
    match value {
        Some(text) => text.into_bound_py_any(py),
        None => Ok(py.None().into_bound(py)),
    }
}

fn optional_metadata<'py>(
    py: Python<'py>,
    value: Option<&MetadataMap>,
) -> PyResult<PythonObject<'py>> {
    match value {
        Some(entries) => json_tree(py, entries),
        None => Ok(py.None().into_bound(py)),
    }
}

fn json_tree<'py>(py: Python<'py>, value: &MetadataMap) -> PyResult<PythonObject<'py>> {
    let json =
        serde_json::to_value(value).map_err(|error| PyValueError::new_err(error.to_string()))?;
    to_object(py, &json)
}

pub async fn run_team_metadata_validation(
    validator: &dyn TeamMetadataValidator,
    payload: &TeamMetadataValidationPayload,
    timeout: ValidationTimeout,
    unavailable_message: &str,
) -> Result<(), TeamMetadataError> {
    if !validator.is_async() {
        return Err(TeamMetadataError::NotAsync);
    }
    let result = validator.invoke(payload, timeout).await;
    finish_validation(result, unavailable_message)
}

pub fn finish_validation(
    result: Result<JsonData, ValidatorFailure>,
    unavailable_message: &str,
) -> Result<(), TeamMetadataError> {
    let value = match result {
        Ok(value) => value,
        Err(failure) => {
            tracing::warn!(error = %failure, "team metadata validation failed");
            return Err(TeamMetadataError::Unavailable {
                message: unavailable_message.to_owned(),
            });
        }
    };
    let parsed = match TeamMetadataValidationResult::from_data(&value) {
        Ok(parsed) => parsed,
        Err(_) => {
            tracing::warn!(
                "the metadata validator returned a result that is not a validation result"
            );
            return Err(TeamMetadataError::Unavailable {
                message: unavailable_message.to_owned(),
            });
        }
    };
    if parsed.valid {
        return Ok(());
    }
    Err(TeamMetadataError::Rejected {
        message: rejection_message(parsed.error_message.as_deref()),
    })
}

// LiteLLM's `or` reads an empty message as missing, while a message of spaces is used as written.
fn rejection_message(error_message: Option<&str>) -> String {
    match error_message {
        Some(message) if !message.is_empty() => message.to_owned(),
        Some(_) | None => DEFAULT_REJECTED_MESSAGE.to_owned(),
    }
}

#[derive(Default)]
pub struct TeamMetadataValidatorRegistry {
    validator: RwLock<Option<Arc<dyn TeamMetadataValidator>>>,
}

impl TeamMetadataValidatorRegistry {
    pub fn set(&self, validator: Option<Arc<dyn TeamMetadataValidator>>) {
        match self.validator.write() {
            Ok(mut current) => *current = validator,
            Err(poisoned) => *poisoned.into_inner() = validator,
        }
    }

    #[must_use]
    pub fn get(&self) -> Option<Arc<dyn TeamMetadataValidator>> {
        match self.validator.read() {
            Ok(current) => (*current).clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[derive(Default)]
pub struct TeamMetadataSchemaRegistry {
    fields: RwLock<Vec<TeamMetadataFieldSchema>>,
}

impl TeamMetadataSchemaRegistry {
    pub fn set(&self, fields: Vec<TeamMetadataFieldSchema>) {
        match self.fields.write() {
            Ok(mut current) => *current = fields,
            Err(poisoned) => *poisoned.into_inner() = fields,
        }
    }

    #[must_use]
    pub fn get(&self) -> Vec<TeamMetadataFieldSchema> {
        match self.fields.read() {
            Ok(current) => (*current).clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

// ast-grep-ignore: no-long-names - LiteLLM names this validate_team_metadata_if_configured
pub async fn validate_team_metadata_if_configured(
    request: MetadataRequest<'_>,
    validators: &TeamMetadataValidatorRegistry,
    settings: &TeamFieldsSettings,
) -> Result<(), TeamMetadataError> {
    let Some(validator) = validators.get() else {
        return Ok(());
    };
    let payload = build_payload(request);
    run_team_metadata_validation(
        validator.as_ref(),
        &payload,
        settings.timeout,
        &settings.unavailable_message,
    )
    .await
}
