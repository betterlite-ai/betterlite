use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use crate::skill_calls::error::{SkillFailure, native_error};
use crate::skill_calls::session::{Call, Mode, SkillCore, SkillOutcome};

type Answer<'py> = PyResult<Bound<'py, PyAny>>;
type Arguments = IndexMap<String, Json>;
type Blobs = Vec<Vec<u8>>;
type Outcome = Result<SkillOutcome, SkillFailure>;

// betterlite.skills.main's module-level entry points, one session owning the client every call goes through.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeSkillSession {
    core: Arc<SkillCore>,
}

fn parse(request: &str, blobs: Blobs) -> PyResult<Call> {
    let arguments: Arguments =
        serde_json::from_str(request).map_err(|error| PyValueError::new_err(error.to_string()))?;
    Ok((arguments, blobs))
}

// The call's answer as Python reads it: the provider's JSON object, or the native error naming its class.
fn native_answer(outcome: Outcome) -> PyResult<String> {
    serde_json::to_string(&outcome.map_err(|failure| native_error(&failure))?)
        .map_err(|error| PyRuntimeError::new_err(error.to_string()))
}

// The awaitable calls: the same attempt on the asyncio loop's runtime, handed back as a coroutine.
fn prompt<'py>(py: Python<'py>, core: Arc<SkillCore>, mode: Mode, call: Call) -> Answer<'py> {
    future_into_py(py, async move {
        let outcome = core.execute(mode, call).await;
        Python::attach(|_attached| native_answer(outcome))
    })
}

#[pymethods]
impl NativeSkillSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        Ok(Self {
            core: Arc::new(SkillCore::build(settings)?),
        })
    }

    fn create_skill(&self, py: Python<'_>, request: &str, blobs: Blobs) -> PyResult<String> {
        let call = parse(request, blobs)?;
        let outcome = py.detach(|| get_runtime().block_on(self.core.execute(Mode::Create, call)));
        native_answer(outcome)
    }

    fn list_skills(&self, py: Python<'_>, request: &str, blobs: Blobs) -> PyResult<String> {
        let call = parse(request, blobs)?;
        let outcome = py.detach(|| get_runtime().block_on(self.core.execute(Mode::List, call)));
        native_answer(outcome)
    }

    fn get_skill(&self, py: Python<'_>, request: &str, blobs: Blobs) -> PyResult<String> {
        let call = parse(request, blobs)?;
        let outcome = py.detach(|| get_runtime().block_on(self.core.execute(Mode::Get, call)));
        native_answer(outcome)
    }

    fn delete_skill(&self, py: Python<'_>, request: &str, blobs: Blobs) -> PyResult<String> {
        let call = parse(request, blobs)?;
        let outcome = py.detach(|| get_runtime().block_on(self.core.execute(Mode::Delete, call)));
        native_answer(outcome)
    }

    fn acreate_skill<'py>(&self, py: Python<'py>, request: &str, blobs: Blobs) -> Answer<'py> {
        let call = parse(request, blobs)?;
        let core = Arc::clone(&self.core);
        prompt(py, core, Mode::Create, call)
    }

    fn alist_skills<'py>(&self, py: Python<'py>, request: &str, blobs: Blobs) -> Answer<'py> {
        let call = parse(request, blobs)?;
        let core = Arc::clone(&self.core);
        prompt(py, core, Mode::List, call)
    }

    fn aget_skill<'py>(&self, py: Python<'py>, request: &str, blobs: Blobs) -> Answer<'py> {
        let call = parse(request, blobs)?;
        let core = Arc::clone(&self.core);
        prompt(py, core, Mode::Get, call)
    }

    fn adelete_skill<'py>(&self, py: Python<'py>, request: &str, blobs: Blobs) -> Answer<'py> {
        let call = parse(request, blobs)?;
        let core = Arc::clone(&self.core);
        prompt(py, core, Mode::Delete, call)
    }
}
