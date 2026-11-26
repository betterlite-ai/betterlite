use std::sync::Arc;

use betterlite_core::api::file_http_upload::service::Mode;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use self::core::FileCallCore;
use super::convert::{Call, parse_create, parse_delete, parse_list, parse_retrieve};

mod core;
mod tests;

type Blobs = Vec<Vec<u8>>;
type Spawned<'py> = PyResult<Bound<'py, PyAny>>;

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeFileCallSession {
    core: Arc<FileCallCore>,
}

#[pymethods]
impl NativeFileCallSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let core = FileCallCore::build(settings)?;
        Ok(Self {
            core: Arc::new(core),
        })
    }

    fn create(&self, py: Python<'_>, payload: &str, blobs: Blobs) -> PyResult<String> {
        let mut owned = blobs;
        let input = parse_create(payload, &mut owned, Mode::Sync)?;
        run_blocking(py, &self.core, Call::Create(Box::new(input)))
    }

    fn acreate<'py>(&self, py: Python<'py>, payload: &str, blobs: Blobs) -> Spawned<'py> {
        let mut owned = blobs;
        let input = parse_create(payload, &mut owned, Mode::Asynchronous)?;
        run_spawned(py, &self.core, Call::Create(Box::new(input)))
    }

    fn delete(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> PyResult<String> {
        let input = parse_delete(payload)?;
        let call = Call::Delete(Box::new(input));
        run_blocking(py, &self.core, call)
    }

    fn adelete<'py>(&self, py: Python<'py>, payload: &str, _blobs: Blobs) -> Spawned<'py> {
        let input = parse_delete(payload)?;
        let call = Call::Delete(Box::new(input));
        run_spawned(py, &self.core, call)
    }

    fn retrieve(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> PyResult<String> {
        let read = parse_retrieve(payload)?;
        let call = Call::Retrieve(read);
        run_blocking(py, &self.core, call)
    }

    fn aretrieve<'py>(&self, py: Python<'py>, payload: &str, _blobs: Blobs) -> Spawned<'py> {
        let read = parse_retrieve(payload)?;
        let call = Call::Retrieve(read);
        run_spawned(py, &self.core, call)
    }

    fn list(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> PyResult<String> {
        let read = parse_list(payload)?;
        let call = Call::List(read);
        run_blocking(py, &self.core, call)
    }

    fn alist<'py>(&self, py: Python<'py>, payload: &str, _blobs: Blobs) -> Spawned<'py> {
        let read = parse_list(payload)?;
        let call = Call::List(read);
        run_spawned(py, &self.core, call)
    }
}

// the type has one inherent impl, so the two ways a call runs are free functions beside it
fn run_blocking(py: Python<'_>, core: &Arc<FileCallCore>, call: Call) -> PyResult<String> {
    let shared = Arc::clone(core);
    let runtime = get_runtime();
    py.detach(move || runtime.block_on(shared.call(&call, Mode::Sync)))
}

fn run_spawned<'py>(py: Python<'py>, core: &Arc<FileCallCore>, call: Call) -> Spawned<'py> {
    let shared = Arc::clone(core);
    future_into_py(
        py,
        async move { shared.call(&call, Mode::Asynchronous).await },
    )
}
