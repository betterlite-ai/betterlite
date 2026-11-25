use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::loader::ConfigError;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::azure_endpoints;
use betterlite_config::sections::resource_sdk_settings::{Raw, RawSettings, Settings};
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::file_content_stream::{
    ContentAnswer, ContentContext, ContentEntry, FileContentStream, StreamedContent,
};
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::files::FilesApi;
use chrono::Utc;
use pyo3::PyErr;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use serde::Deserialize;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::convert::{ConvertError, FileCall, PlainAnswer, StreamAnswer, parse_call};
use super::dispatch::ContentDispatch;
use super::error::failure_for;
use super::stream::NativeFileStream;

mod tests;

// Everything one Python file content session owns: the environment snapshot, the settings and the client.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SessionWire {
    environment: Option<BTreeMap<String, String>>,
    settings: RawSettings,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error("the file content settings are not valid JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the proxy environment could not be read: {source}")]
    Proxy {
        #[source]
        source: EnvError,
    },
    #[error("the outbound http client could not be built: {source}")]
    Client {
        #[source]
        source: HttpError,
    },
    #[error("the file content settings could not be read: {source}")]
    Settings {
        #[source]
        source: ConfigError,
    },
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        PyValueError::new_err(error.to_string())
    }
}

pub(crate) struct FileContentCore {
    client: ProxiedClient,
    settings: Settings,
    environment: BTreeMap<String, String>,
    endpoints: azure_endpoints::Settings,
    catalog: Arc<ModelCatalog>,
    known_models: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    registry: JSONProviderRegistry,
    policies: ProviderPolicies,
    globals: GlobalCredentials,
}

impl FileContentCore {
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire =
            serde_json::from_str(text).map_err(|source| SessionError::Json { source })?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let settings = Raw {
            settings: wire.settings,
        }
        .convert(&environment)
        .map_err(|source| SessionError::Settings { source })?;
        let endpoints = azure_endpoints::Raw::default()
            .convert(&environment)
            .map_err(|source| SessionError::Settings { source })?;
        let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment)
            .map_err(|source| SessionError::Proxy { source })?;
        let client = ProxiedClient::outbound(&Arc::new(policy))
            .map_err(|source| SessionError::Client { source })?;
        let globals = GlobalCredentials {
            api_key: settings.api_key.clone(),
            api_base: settings.api_base.clone(),
            xai_key: None,
            inception_key: None,
            lemonade_key: None,
        };
        Ok(Self {
            client,
            settings,
            environment,
            endpoints,
            catalog: BundledCatalog::default().get(),
            known_models: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals,
        })
    }

    fn dispatch(&self) -> ContentDispatch {
        ContentDispatch {
            openai: FilesApi {
                transport: self.client.clone(),
                boundaries: Uuid::new_v4,
            },
            client: self.client.clone(),
            environment: self.environment.clone(),
            endpoints: self.endpoints.clone(),
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: self.catalog.as_ref(),
            known_model_names: &self.known_models,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: self.policies.clone(),
            globals: self.globals.clone(),
        }
    }

    pub(crate) async fn content(
        &self,
        entry: ContentEntry,
        call: &FileCall,
    ) -> Result<ContentAnswer, PyErr> {
        let resolution = self.resolution();
        let service = FileContentStream {
            dispatch: self.dispatch(),
            settings: self.settings.clone(),
            resolution: &resolution,
            clock: || Utc::now().fixed_offset(),
            call_ids: Uuid::new_v4,
        };
        let context = ContentContext {
            trusted: None,
            client: None,
            logger: None,
            logging: None,
        };
        service
            .run(entry, &call.call(), context)
            .await
            .map_err(|error| PyErr::from(failure_for(&error)))
    }
}

// The answer Python reads: the JSON text of the headers, the blobs beside it, and the stream when there is one.
fn answer_tuple(py: Python<'_>, answer: ContentAnswer, is_async: bool) -> PyResult<Py<PyAny>> {
    let (text, blobs, stream) = match answer {
        ContentAnswer::Buffered(content) => {
            let plain = PlainAnswer {
                status: content.status,
                headers: content.headers.into_iter().collect(),
            };
            let text = serde_json::to_string(&plain)
                .map_err(|source| PyErr::from(ConvertError::Write { source }))?;
            let blobs = vec![PyBytes::new(py, &content.body).unbind()];
            (text, blobs, None::<Py<PyAny>>)
        }
        ContentAnswer::Streamed(content) => stream_text(py, *content, is_async)?,
    };
    (text, blobs, stream).into_pyobject(py)?.into_any().unbind()
}

type StreamField = (String, Vec<Py<PyBytes>>, Option<Py<PyAny>>);

fn stream_text(
    py: Python<'_>,
    content: StreamedContent,
    is_async: bool,
) -> Result<StreamField, PyErr> {
    let plain = StreamAnswer {
        headers: content.headers.into_iter().collect(),
    };
    let text = serde_json::to_string(&plain)
        .map_err(|source| PyErr::from(ConvertError::Write { source }))?;
    let stream = NativeFileStream {
        stream: Arc::new(Mutex::new(content.response)),
        is_async,
    };
    Ok((text, Vec::new(), Some(Py::new(py, stream)?.into_any())))
}

// main.py's file_content and afile_content, one session owning the environment snapshot every call reads.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeFileContentSession {
    core: Arc<FileContentCore>,
}

#[pymethods]
impl NativeFileContentSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let core = FileContentCore::build(settings)?;
        Ok(Self {
            core: Arc::new(core),
        })
    }

    // the synchronous download: the provider call is already awaitable, so the runtime drives it here
    fn content(&self, py: Python<'_>, payload: &str, _blobs: Vec<Vec<u8>>) -> PyResult<Py<PyAny>> {
        let call = parse_call(payload)?;
        let core = Arc::clone(&self.core);
        let answer =
            py.detach(move || get_runtime().block_on(core.content(ContentEntry::Sync, &call)))?;
        answer_tuple(py, answer, false)
    }

    // the awaitable download, handed back as a coroutine the caller awaits
    fn acontent<'py>(
        &self,
        py: Python<'py>,
        payload: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call = parse_call(payload)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, async move {
            let answer = core.content(ContentEntry::Async, &call).await?;
            Ok(Python::attach(|attached| answer_tuple(attached, answer, true))?)
        })
    }
}
