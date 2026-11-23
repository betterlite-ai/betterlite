use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use serde::Serialize;

use crate::skill_calls::convert;
use crate::skill_calls::error::{SkillFailure, native_error};
use crate::skill_calls::session::SkillCore;

// betterlite.skills.main's module-level entry points, one session owning the client every call goes through.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeSkillSession {
    core: Arc<SkillCore>,
}

impl NativeSkillSession {
    fn arguments(request: &str) -> PyResult<IndexMap<String, Json>> {
        convert::parse(request).map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn answer<T: Serialize>(outcome: Result<T, SkillFailure>) -> PyResult<String> {
        let value = outcome.map_err(|failure| native_error(&failure))?;
        serde_json::to_string(&value).map_err(|error| PyRuntimeError::new_err(error.to_string()))
    }
}

#[pymethods]
impl NativeSkillSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        Ok(Self {
            core: Arc::new(SkillCore::build(settings)?),
        })
    }

    // the synchronous calls: one attempt on the runtime, blocking this thread with the GIL released
    fn create_skill(&self, py: Python<'_>, request: &str, blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let arguments = Self::arguments(request)?;
        let outcome = py.detach(|| get_runtime().block_on(self.core.create(arguments, blobs)));
        Self::answer(outcome)
    }

    fn list_skills(&self, py: Python<'_>, request: &str, _blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let arguments = Self::arguments(request)?;
        let outcome = py.detach(|| get_runtime().block_on(self.core.list(arguments)));
        Self::answer(outcome)
    }

    fn get_skill(&self, py: Python<'_>, request: &str, _blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let arguments = Self::arguments(request)?;
        let outcome = py.detach(|| get_runtime().block_on(self.core.get(arguments)));
        Self::answer(outcome)
    }

    fn delete_skill(
        &self,
        py: Python<'_>,
        request: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<String> {
        let arguments = Self::arguments(request)?;
        let outcome = py.detach(|| get_runtime().block_on(self.core.delete(arguments)));
        Self::answer(outcome)
    }

    // the awaitable calls: the same call on the asyncio loop's runtime, handed back as a coroutine
    fn acreate_skill<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let arguments = Self::arguments(request)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let outcome = core.create(arguments, blobs).await;
            Python::attach(|_attached| Self::answer(outcome))
        })
    }

    fn alist_skills<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let arguments = Self::arguments(request)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let outcome = core.list(arguments).await;
            Python::attach(|_attached| Self::answer(outcome))
        })
    }

    fn aget_skill<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let arguments = Self::arguments(request)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let outcome = core.get(arguments).await;
            Python::attach(|_attached| Self::answer(outcome))
        })
    }

    fn adelete_skill<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let arguments = Self::arguments(request)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let outcome = core.delete(arguments).await;
            Python::attach(|_attached| Self::answer(outcome))
        })
    }
}
