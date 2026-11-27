use std::sync::Arc;
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::loader::startup::process_environment;
use betterlite_context::core_settings::CoreSettings;
use betterlite_http::proxy::ProxiedClient;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use self::operations::Mode;
use super::configs::EvalConfigs;
use super::convert::{SessionWire, parse};
use super::error::{EvalOperationFailure, SessionError};
use crate::completion::setup::outbound_client;

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
        let configs = EvalConfigs::new(&wire.settings, &environment);
        Ok(Self {
            configs,
            client,
            core: CoreSettings {
                request_timeout_seconds: timeout.seconds,
                request_timeout_explicit: timeout.explicitly_set,
                litellm_request_timeout: None,
                model_timeout_overrides: Vec::new(),
            },
            redactor,
            request_timeout,
        })
    }
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
        let outcome =
            py.detach(|| get_runtime().block_on(self.core.execute(Mode::Create, request)));
        outcome.map_err(EvalOperationFailure::into_py)
    }

    fn acreate_eval<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let core = Arc::clone(&self.core);
        let request = request.to_owned();
        future_into_py(py, async move {
            let outcome = core.execute(Mode::Create, &request).await;
            Python::attach(|_attached| outcome.map_err(EvalOperationFailure::into_py))
        })
    }

    fn list_evals(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let outcome = py.detach(|| get_runtime().block_on(self.core.execute(Mode::List, request)));
        outcome.map_err(EvalOperationFailure::into_py)
    }

    fn alist_evals<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let core = Arc::clone(&self.core);
        let request = request.to_owned();
        future_into_py(py, async move {
            let outcome = core.execute(Mode::List, &request).await;
            Python::attach(|_attached| outcome.map_err(EvalOperationFailure::into_py))
        })
    }

    fn get_eval(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let outcome = py.detach(|| get_runtime().block_on(self.core.execute(Mode::Get, request)));
        outcome.map_err(EvalOperationFailure::into_py)
    }

    fn aget_eval<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let core = Arc::clone(&self.core);
        let request = request.to_owned();
        future_into_py(py, async move {
            let outcome = core.execute(Mode::Get, &request).await;
            Python::attach(|_attached| outcome.map_err(EvalOperationFailure::into_py))
        })
    }

    fn update_eval(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let outcome =
            py.detach(|| get_runtime().block_on(self.core.execute(Mode::Update, request)));
        outcome.map_err(EvalOperationFailure::into_py)
    }

    fn aupdate_eval<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let core = Arc::clone(&self.core);
        let request = request.to_owned();
        future_into_py(py, async move {
            let outcome = core.execute(Mode::Update, &request).await;
            Python::attach(|_attached| outcome.map_err(EvalOperationFailure::into_py))
        })
    }

    fn delete_eval(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let outcome =
            py.detach(|| get_runtime().block_on(self.core.execute(Mode::Delete, request)));
        outcome.map_err(EvalOperationFailure::into_py)
    }

    fn adelete_eval<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let core = Arc::clone(&self.core);
        let request = request.to_owned();
        future_into_py(py, async move {
            let outcome = core.execute(Mode::Delete, &request).await;
            Python::attach(|_attached| outcome.map_err(EvalOperationFailure::into_py))
        })
    }

    fn cancel_eval(&self, py: Python<'_>, request: &str, _blobs: Blobs) -> PyResult<String> {
        let outcome =
            py.detach(|| get_runtime().block_on(self.core.execute(Mode::Cancel, request)));
        outcome.map_err(EvalOperationFailure::into_py)
    }

    fn acancel_eval<'py>(&self, py: Python<'py>, request: &str, _blobs: Blobs) -> Answer<'py> {
        let core = Arc::clone(&self.core);
        let request = request.to_owned();
        future_into_py(py, async move {
            let outcome = core.execute(Mode::Cancel, &request).await;
            Python::attach(|_attached| outcome.map_err(EvalOperationFailure::into_py))
        })
    }
}
