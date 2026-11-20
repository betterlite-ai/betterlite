use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
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

use super::metadata::{MetadataMap, TeamMetadataValidationPayload};
use super::validation::TeamMetadataValidator;

type PythonObject<'py> = Bound<'py, PyAny>;

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
