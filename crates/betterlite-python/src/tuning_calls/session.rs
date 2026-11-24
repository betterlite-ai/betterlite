use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::resource_sdk_settings::{Raw, Settings};
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::fine_tuning_cancel_call::FineTuningCancel;
use betterlite_core::api::fine_tuning_create::service::FineTuningCreate;
use betterlite_core::api::fine_tuning_read::FineTuningReadOutcome;
use betterlite_core::api::fine_tuning_read::service::FineTuningRead;
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_http::proxy::ProxiedClient;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use serde::de::DeserializeOwned;

use super::convert::failures::{TuningFailure, wire_failure};
use super::convert::{CancelCall, CreateCall, NativeCall, ReadCall, SettingsWire};
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

    // The module settings a call carries, read against the environment snapshot this session took.
    fn settings_of(&self, wire: SettingsWire) -> Result<Settings, Raised> {
        Raw { settings: wire.raw }
            .convert(&self.environment)
            .map_err(|source| wire_failure(WireError::Config { source }))
    }

    async fn create(&self, call: CreateCall) -> Result<String, Raised> {
        let dispatch = CreateDispatch {
            inner: self.dispatch.clone(),
            integrations: call.call.integrations.clone(),
        };
        let mut request = call.call;
        // the core record refuses the loose list[str], so it stays out of the call it builds
        request.integrations = None;
        let settings = self.settings_of(call.settings)?;
        let service = FineTuningCreate {
            dispatch,
            settings: resource_sdk(settings),
        };
        let job = service
            .create(&request)
            .await
            .map_err(TuningFailure::Create)?;
        serde_json::to_string(&job).map_err(|source| wire_failure(WireError::Encode { source }))
    }

    // Every native call crosses here: one dispatch, then the operation the call names.
    pub(crate) async fn execute(&self, call: NativeCall) -> Result<String, Raised> {
        match call {
            NativeCall::Create(create) => self.create(*create).await,
            NativeCall::Read(read) => {
                let ReadCall { input, settings } = *read;
                let service = FineTuningRead {
                    dispatch: self.dispatch.clone(),
                    settings: self.settings_of(settings)?,
                };
                let outcome = service.run(&input).await.map_err(TuningFailure::Read)?;
                read_text(outcome)
            }
            NativeCall::Cancel(cancel) => {
                let CancelCall {
                    call: request,
                    settings,
                } = *cancel;
                let service = FineTuningCancel {
                    dispatch: self.dispatch.clone(),
                    settings: self.settings_of(settings)?,
                };
                let job = service.run(&request).await.map_err(TuningFailure::Cancel)?;
                serde_json::to_string(&job)
                    .map_err(|source| wire_failure(WireError::Encode { source }))
            }
        }
    }
}

// The page or the job a read call resolved, written as the record Python validates.
fn read_text(outcome: FineTuningReadOutcome) -> Result<String, Raised> {
    match outcome {
        FineTuningReadOutcome::Page(page) => serde_json::to_string(&page),
        FineTuningReadOutcome::Job(job) => serde_json::to_string(&job),
    }
    .map_err(|source| wire_failure(WireError::Encode { source }))
}

// The request text of one call, decoded into the call type its method owns.
fn decode<T>(text: &str) -> PyResult<T>
where
    T: DeserializeOwned,
{
    match serde_json::from_str(text) {
        Ok(call) => Ok(call),
        Err(source) => Err(PyValueError::new_err(source.to_string())),
    }
}

// The awaitable answer: the same call on the asyncio loop's runtime, handed back as a coroutine.
fn async_answer(py: Python<'_>, core: Arc<TuningCore>, call: NativeCall) -> PyResult<Py<PyAny>> {
    let future = future_into_py(py, async move {
        let text = core.execute(call).await.map_err(Raised::into_pyerr)?;
        Ok::<String, PyErr>(text)
    })?;
    Ok(future.unbind())
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
        let core = TuningCore::build().map_err(Raised::into_pyerr)?;
        Ok(Self {
            core: Arc::new(core),
        })
    }

    fn create(&self, py: Python<'_>, raw: &str, _blobs: Py<PyAny>) -> PyResult<String> {
        let call: CreateCall = decode(raw)?;
        py.detach(|| get_runtime().block_on(self.core.execute(NativeCall::Create(Box::new(call)))))
            .map_err(Raised::into_pyerr)
    }

    fn acreate(&self, py: Python<'_>, raw: &str, _blobs: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let call: CreateCall = decode(raw)?;
        let core = Arc::clone(&self.core);
        async_answer(py, core, NativeCall::Create(Box::new(call)))
    }

    fn retrieve(&self, py: Python<'_>, raw: &str, _blobs: Py<PyAny>) -> PyResult<String> {
        let call: ReadCall = decode(raw)?;
        py.detach(|| get_runtime().block_on(self.core.execute(NativeCall::Read(Box::new(call)))))
            .map_err(Raised::into_pyerr)
    }

    fn aretrieve(&self, py: Python<'_>, raw: &str, _blobs: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let call: ReadCall = decode(raw)?;
        let core = Arc::clone(&self.core);
        async_answer(py, core, NativeCall::Read(Box::new(call)))
    }

    fn list(&self, py: Python<'_>, raw: &str, _blobs: Py<PyAny>) -> PyResult<String> {
        let call: ReadCall = decode(raw)?;
        py.detach(|| get_runtime().block_on(self.core.execute(NativeCall::Read(Box::new(call)))))
            .map_err(Raised::into_pyerr)
    }

    fn alist(&self, py: Python<'_>, raw: &str, _blobs: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let call: ReadCall = decode(raw)?;
        let core = Arc::clone(&self.core);
        async_answer(py, core, NativeCall::Read(Box::new(call)))
    }

    fn cancel(&self, py: Python<'_>, raw: &str, _blobs: Py<PyAny>) -> PyResult<String> {
        let call: CancelCall = decode(raw)?;
        py.detach(|| get_runtime().block_on(self.core.execute(NativeCall::Cancel(Box::new(call)))))
            .map_err(Raised::into_pyerr)
    }

    fn acancel(&self, py: Python<'_>, raw: &str, _blobs: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let call: CancelCall = decode(raw)?;
        let core = Arc::clone(&self.core);
        async_answer(py, core, NativeCall::Cancel(Box::new(call)))
    }
}

// The core create service reads ResourceSdkSettings, and every section field maps to the same one.
fn resource_sdk(settings: Settings) -> ResourceSdkSettings {
    ResourceSdkSettings {
        api_base: settings.api_base,
        api_version: settings.api_version,
        api_key: settings.api_key,
        openai_key: settings.openai_key,
        azure_key: settings.azure_key,
        azure_ad_token: settings.azure_ad_token,
        organization: settings.organization,
        vertex_project: settings.vertex_project,
        vertex_location: settings.vertex_location,
        openai_base_url: settings.openai_base_url,
        openai_api_base: settings.openai_api_base,
        openai_api_key: settings.openai_api_key,
        openai_organization: settings.openai_organization,
        azure_api_base: settings.azure_api_base,
        azure_api_version: settings.azure_api_version,
        azure_openai_api_key: settings.azure_openai_api_key,
        azure_api_key: settings.azure_api_key,
        vertexai_project: settings.vertexai_project,
        vertexai_location: settings.vertexai_location,
        vertexai_credentials: settings.vertexai_credentials,
    }
}
