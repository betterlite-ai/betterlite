mod convert;
mod core;
mod error;
mod stack;
mod transport;

use std::future::Future;
use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes, PyType};
use pyo3_async_runtimes::tokio::future_into_py;

use self::convert::{CallWire, parse};
use self::core::{Answer, Fallible, SessionCore, json_text};

type Awaitable<'py> = PyResult<Bound<'py, PyAny>>;

// litellm.containers.main's file list and upload plus endpoint_factory.py's three generated calls
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let base = module.getattr("NativeError")?;
    let base_type = base.cast::<PyType>()?;
    let error_type = PyErr::new_type(
        module.py(),
        c"betterlite._native.NativeContainerFileError",
        None,
        Some(base_type),
        None,
    )?;
    module.add("NativeContainerFileError", error_type)?;
    module.add_class::<NativeContainerFileSession>()?;
    Ok(())
}

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeContainerFileSession {
    core: Arc<SessionCore>,
    error_type: Py<PyType>,
}

// the awaitable half of a call: the work is a future already, and only the native error needs the GIL
fn await_answer<'py, T, F>(py: Python<'py>, error_type: Py<PyType>, work: F) -> Awaitable<'py>
where
    T: for<'any> IntoPyObject<'any> + Send + 'static,
    F: Future<Output = Fallible<T>> + Send + 'static,
{
    future_into_py(py, async move {
        let answer = work.await;
        Python::attach(|attached| {
            answer.map_err(|failure| failure.into_pyerr(attached, &error_type))
        })
    })
}

// a content answer is the provider's bytes, or the catalog's parsed mock value
fn answer_object(py: Python<'_>, answer: Answer) -> PyResult<Bound<'_, PyAny>> {
    match answer {
        Answer::Bytes(bytes) => Ok(PyBytes::new(py, &bytes).into_any()),
        Answer::Json(text) => py.import("json")?.call_method1("loads", (text,)),
    }
}

impl<'py> IntoPyObject<'py> for Answer {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        answer_object(py, self)
    }
}

#[pymethods]
impl NativeContainerFileSession {
    #[new]
    fn new(py: Python<'_>, settings: &str) -> PyResult<Self> {
        let core = SessionCore::build(settings)?;
        let error_type = py
            .import("betterlite._native")?
            .getattr("NativeContainerFileError")?
            .cast_into::<PyType>()?
            .unbind();
        Ok(Self {
            core: Arc::new(core),
            error_type,
        })
    }

    // list and upload answer JSON; the file bytes the caller sent sit beside the request text
    fn call(&self, py: Python<'_>, request: &str, blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let wire: CallWire = parse(request)?;
        let answer = py
            .detach(|| self.core.call(&wire, blobs))
            .map_err(|failure| failure.into_pyerr(py, &self.error_type))?;
        json_text(answer, &wire).map_err(|failure| failure.into_pyerr(py, &self.error_type))
    }

    // retrieve_container_file_content answers the provider's bytes, or a mock as parsed JSON
    fn content(&self, py: Python<'_>, request: &str) -> PyResult<Py<PyAny>> {
        let wire: CallWire = parse(request)?;
        let answer = py
            .detach(|| self.core.call(&wire, Vec::new()))
            .map_err(|failure| failure.into_pyerr(py, &self.error_type))?;
        Ok(answer_object(py, answer)?.unbind())
    }

    fn acall<'py>(&self, py: Python<'py>, request: &str, blobs: Vec<Vec<u8>>) -> Awaitable<'py> {
        let wire: CallWire = parse(request)?;
        let core = Arc::clone(&self.core);
        let work = async move {
            let answer = core.dispatch(&wire, blobs).await?;
            json_text(answer, &wire)
        };
        await_answer(py, self.error_type.clone_ref(py), work)
    }

    fn acontent<'py>(&self, py: Python<'py>, request: &str) -> Awaitable<'py> {
        let wire: CallWire = parse(request)?;
        let core = Arc::clone(&self.core);
        await_answer(py, self.error_type.clone_ref(py), async move {
            core.dispatch(&wire, Vec::new()).await
        })
    }
}
