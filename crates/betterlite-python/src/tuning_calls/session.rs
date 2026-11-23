use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::fine_tuning_cancel_call::FineTuningCancel;
use betterlite_core::api::fine_tuning_create::service::FineTuningCreate;
use betterlite_core::api::fine_tuning_read::service::FineTuningRead;
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::fine_tuning::job::FineTuneJob;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::convert::failures::{cancel_failure, create_failure, read_failure, wire_failure};
use super::convert::{CancelCall, CreateCall, ReadCall, parse, request_error};
use super::error::{Raised, WireError};
use super::http::{CreateDispatch, WireDispatch};

// Everything one Python session owns: the environment snapshot and the HTTP client every call goes through.
pub(crate) struct TuningCore {
    environment: BTreeMap<String, String>,
    dispatch: WireDispatch,
}

impl TuningCore {
    fn build() -> Result<Self, Raised> {
        let environment = process_environment();
        let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment)
            .map_err(|source| wire_failure(WireError::Proxy { source }))?;
        let client = ProxiedClient::outbound(&Arc::new(policy))
            .map_err(|source| wire_failure(WireError::Client { source }))?;
        Ok(Self {
            environment,
            dispatch: WireDispatch { client },
        })
    }

    pub(crate) async fn create(&self, mut call: CreateCall) -> Result<String, Raised> {
        let dispatch = CreateDispatch {
            inner: self.dispatch.clone(),
            integrations: call.call.integrations.clone(),
        };
        // the core record refuses the loose list[str], so it stays out of the call it builds
        call.call.integrations = None;
        let settings = call
            .settings
            .configured(&self.environment)
            .map_err(|source| wire_failure(WireError::Config { source }))?;
        let job = FineTuningCreate {
            dispatch,
            settings: ResourceSdkSettings::from(&settings),
        }
        .create(&call.call)
        .await
        .map_err(create_failure)?;
        job_text(&job)
    }

    pub(crate) async fn read(&self, call: ReadCall) -> Result<String, Raised> {
        let settings = call
            .settings
            .configured(&self.environment)
            .map_err(|source| wire_failure(WireError::Config { source }))?;
        let outcome = FineTuningRead {
            dispatch: self.dispatch.clone(),
            settings,
        }
        .run(&call.input)
        .await
        .map_err(read_failure)?;
        serde_json::to_string(&outcome).map_err(|source| wire_failure(WireError::Encode { source }))
    }

    pub(crate) async fn cancel(&self, call: CancelCall) -> Result<String, Raised> {
        let settings = call
            .settings
            .configured(&self.environment)
            .map_err(|source| wire_failure(WireError::Config { source }))?;
        let job = FineTuningCancel {
            dispatch: self.dispatch.clone(),
            settings,
        }
        .run(&call.call)
        .await
        .map_err(cancel_failure)?;
        job_text(&job)
    }
}

fn job_text(job: &FineTuneJob) -> Result<String, Raised> {
    serde_json::to_string(job).map_err(|source| wire_failure(WireError::Encode { source }))
}

// fine_tuning.main's module-level entry points, one session owning the client every call goes through.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeTuningSession {
    core: Arc<TuningCore>,
}

#[pymethods]
impl NativeTuningSession {
    #[new]
    fn new() -> PyResult<Self> {
        let core = TuningCore::build().map_err(Raised::to_pyerr)?;
        Ok(Self {
            core: Arc::new(core),
        })
    }

    // the synchronous call: one attempt on the runtime, blocking this thread with the GIL released
    fn create(&self, py: Python<'_>, request: &str, _blobs: &Bound<'_, PyAny>) -> PyResult<String> {
        let call: CreateCall = parse(request).map_err(request_error)?;
        py.detach(|| get_runtime().block_on(self.core.create(call)))
            .map_err(Raised::to_pyerr)
    }

    // the awaitable call: the same request on the asyncio loop's runtime, handed back as a coroutine
    fn acreate<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call: CreateCall = parse(request).map_err(request_error)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let text = core.create(call).await.map_err(Raised::to_pyerr)?;
            Ok::<String, PyErr>(text)
        })
    }

    fn retrieve(
        &self,
        py: Python<'_>,
        request: &str,
        _blobs: &Bound<'_, PyAny>,
    ) -> PyResult<String> {
        let call: ReadCall = parse(request).map_err(request_error)?;
        py.detach(|| get_runtime().block_on(self.core.read(call)))
            .map_err(Raised::to_pyerr)
    }

    fn aretrieve<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call: ReadCall = parse(request).map_err(request_error)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let text = core.read(call).await.map_err(Raised::to_pyerr)?;
            Ok::<String, PyErr>(text)
        })
    }

    fn list(&self, py: Python<'_>, request: &str, _blobs: &Bound<'_, PyAny>) -> PyResult<String> {
        let call: ReadCall = parse(request).map_err(request_error)?;
        py.detach(|| get_runtime().block_on(self.core.read(call)))
            .map_err(Raised::to_pyerr)
    }

    fn alist<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call: ReadCall = parse(request).map_err(request_error)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let text = core.read(call).await.map_err(Raised::to_pyerr)?;
            Ok::<String, PyErr>(text)
        })
    }

    fn cancel(&self, py: Python<'_>, request: &str, _blobs: &Bound<'_, PyAny>) -> PyResult<String> {
        let call: CancelCall = parse(request).map_err(request_error)?;
        py.detach(|| get_runtime().block_on(self.core.cancel(call)))
            .map_err(Raised::to_pyerr)
    }

    fn acancel<'py>(
        &self,
        py: Python<'py>,
        request: &str,
        _blobs: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call: CancelCall = parse(request).map_err(request_error)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let text = core.cancel(call).await.map_err(Raised::to_pyerr)?;
            Ok::<String, PyErr>(text)
        })
    }
}
