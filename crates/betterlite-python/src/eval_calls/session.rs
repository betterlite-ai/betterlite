use std::sync::Arc;
use std::time::Duration;

use betterlite_context::core_settings::CoreSettings;
use betterlite_http::proxy::ProxiedClient;
use betterlite_redact::secret::Redactor;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use self::operations::{EvalCall, RequestCall};
use super::configs::EvalConfigs;
use super::error::EvalOperationFailure;
use super::payload::{CreatePayload, IdPayload, ListPayload, UpdatePayload};

mod logging;
mod operations;
mod tests;

type Answer<'py> = PyResult<Bound<'py, PyAny>>;
type Blobs = Vec<Vec<u8>>;

// main.py's eval calls: one session holding the pooled client, the provider config and the resolved request timeout.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeEvalSession {
    core: Arc<EvalCore>,
}

// Everything one Python eval session owns: the provider config, the pooled client, B198's timeout and B187's redactor.
pub(super) struct EvalCore {
    configs: EvalConfigs,
    client: ProxiedClient,
    core: CoreSettings,
    redactor: Redactor,
    request_timeout: Duration,
}

#[pymethods]
impl NativeEvalSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        Ok(Self {
            core: Arc::new(EvalCore::build(settings)?),
        })
    }

    fn create_eval(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let payload = CreatePayload::parse(request).map_err(EvalOperationFailure::from)?;
        let call = EvalCall::Request(Box::new(RequestCall::Create(payload)));
        run_blocking(py, &self.core, call)
    }

    fn acreate_eval<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let payload = CreatePayload::parse(request).map_err(EvalOperationFailure::from)?;
        let call = EvalCall::Request(Box::new(RequestCall::Create(payload)));
        run_spawned(py, &self.core, call)
    }

    fn list_evals(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let payload = ListPayload::parse(request).map_err(EvalOperationFailure::from)?;
        let call = EvalCall::Request(Box::new(RequestCall::List(payload)));
        run_blocking(py, &self.core, call)
    }

    fn alist_evals<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let payload = ListPayload::parse(request).map_err(EvalOperationFailure::from)?;
        let call = EvalCall::Request(Box::new(RequestCall::List(payload)));
        run_spawned(py, &self.core, call)
    }

    fn get_eval(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let payload = IdPayload::parse(request).map_err(EvalOperationFailure::from)?;
        let call = EvalCall::Request(Box::new(RequestCall::Get(payload)));
        run_blocking(py, &self.core, call)
    }

    fn aget_eval<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let payload = IdPayload::parse(request).map_err(EvalOperationFailure::from)?;
        let call = EvalCall::Request(Box::new(RequestCall::Get(payload)));
        run_spawned(py, &self.core, call)
    }

    fn update_eval(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let payload = UpdatePayload::parse(request).map_err(EvalOperationFailure::from)?;
        let call = EvalCall::Request(Box::new(RequestCall::Update(payload)));
        run_blocking(py, &self.core, call)
    }

    fn aupdate_eval<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let payload = UpdatePayload::parse(request).map_err(EvalOperationFailure::from)?;
        let call = EvalCall::Request(Box::new(RequestCall::Update(payload)));
        run_spawned(py, &self.core, call)
    }

    fn delete_eval(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let payload = IdPayload::parse(request).map_err(EvalOperationFailure::from)?;
        let call = EvalCall::Delete(payload);
        run_blocking(py, &self.core, call)
    }

    fn adelete_eval<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let payload = IdPayload::parse(request).map_err(EvalOperationFailure::from)?;
        let call = EvalCall::Delete(payload);
        run_spawned(py, &self.core, call)
    }

    fn cancel_eval(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let payload = IdPayload::parse(request).map_err(EvalOperationFailure::from)?;
        let call = EvalCall::Cancel(payload);
        run_blocking(py, &self.core, call)
    }

    fn acancel_eval<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let payload = IdPayload::parse(request).map_err(EvalOperationFailure::from)?;
        let call = EvalCall::Cancel(payload);
        run_spawned(py, &self.core, call)
    }
}

fn run_blocking(py: Python<'_>, core: &Arc<EvalCore>, call: EvalCall) -> PyResult<String> {
    let shared = Arc::clone(core);
    let runtime = get_runtime();
    py.detach(move || runtime.block_on(shared.execute(call)))
        .map_err(PyErr::from)
}

fn run_spawned<'py>(py: Python<'py>, core: &Arc<EvalCore>, call: EvalCall) -> Answer<'py> {
    let shared = Arc::clone(core);
    future_into_py(py, async move {
        let outcome = shared.execute(call).await;
        Python::attach(|_attached| outcome.map_err(PyErr::from))
    })
}
