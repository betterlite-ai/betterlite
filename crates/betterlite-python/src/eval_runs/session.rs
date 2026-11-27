use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::loader::startup::process_environment;
use betterlite_http::proxy::ProxiedClient;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use self::operations::RunCall;
use super::configs::{ModuleSettings, SessionWire};
use super::convert::{CreateRunPayload, ListRunPayload, RunIdPayload, parse};
use super::error::SessionError;
use crate::completion::setup::outbound_client;

mod logging;
mod operations;
mod tests;

type Answer<'py> = PyResult<Bound<'py, PyAny>>;
type Blobs = Vec<Vec<u8>>;

// _runs.py's run calls: one session holding the pooled client, the provider config and the resolved request timeout.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeEvalRunSession {
    core: Arc<EvalCore>,
}

// Everything one Python run session owns: the settings its configs are built from, the pooled client, B198's timeout and B187's redactor.
pub(super) struct EvalCore {
    settings: ModuleSettings,
    environment: BTreeMap<String, String>,
    client: ProxiedClient,
    redactor: Redactor,
    request_timeout: Duration,
}

impl EvalCore {
    pub(super) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire = parse(text)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let client = outbound_client(&environment)?;
        let timeout = RequestTimeout::resolve(&environment)?;
        let request_timeout = Duration::try_from_secs_f64(timeout.seconds).map_err(|source| {
            SessionError::Duration {
                seconds: timeout.seconds,
                source,
            }
        })?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
        Ok(Self {
            settings: wire.settings,
            environment,
            client,
            redactor,
            request_timeout,
        })
    }
}

#[pymethods]
impl NativeEvalRunSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        Ok(Self {
            core: Arc::new(EvalCore::build(settings)?),
        })
    }

    fn create_run(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let payload = CreateRunPayload::parse(request)?;
        let call = RunCall::Create(Box::new(payload));
        run_blocking(py, &self.core, call)
    }

    fn acreate_run<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let payload = CreateRunPayload::parse(request)?;
        let call = RunCall::Create(Box::new(payload));
        run_spawned(py, &self.core, call)
    }

    fn list_runs(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let payload = ListRunPayload::parse(request)?;
        let call = RunCall::List(Box::new(payload));
        run_blocking(py, &self.core, call)
    }

    fn alist_runs<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let payload = ListRunPayload::parse(request)?;
        let call = RunCall::List(Box::new(payload));
        run_spawned(py, &self.core, call)
    }

    fn get_run(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let payload = RunIdPayload::parse(request)?;
        let call = RunCall::Get(Box::new(payload));
        run_blocking(py, &self.core, call)
    }

    fn aget_run<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let payload = RunIdPayload::parse(request)?;
        let call = RunCall::Get(Box::new(payload));
        run_spawned(py, &self.core, call)
    }

    fn cancel_run(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let payload = RunIdPayload::parse(request)?;
        let call = RunCall::Cancel(Box::new(payload));
        run_blocking(py, &self.core, call)
    }

    fn acancel_run<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let payload = RunIdPayload::parse(request)?;
        let call = RunCall::Cancel(Box::new(payload));
        run_spawned(py, &self.core, call)
    }

    fn delete_run(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let payload = RunIdPayload::parse(request)?;
        let call = RunCall::Delete(Box::new(payload));
        run_blocking(py, &self.core, call)
    }

    fn adelete_run<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let payload = RunIdPayload::parse(request)?;
        let call = RunCall::Delete(Box::new(payload));
        run_spawned(py, &self.core, call)
    }
}

fn run_blocking(py: Python<'_>, core: &Arc<EvalCore>, call: RunCall) -> PyResult<String> {
    let shared = Arc::clone(core);
    let runtime = get_runtime();
    py.detach(move || runtime.block_on(shared.execute(call)))
        .map_err(PyErr::from)
}

fn run_spawned<'py>(py: Python<'py>, core: &Arc<EvalCore>, call: RunCall) -> Answer<'py> {
    let shared = Arc::clone(core);
    future_into_py(py, async move {
        let outcome = shared.execute(call).await;
        Python::attach(|_attached| outcome.map_err(PyErr::from))
    })
}
