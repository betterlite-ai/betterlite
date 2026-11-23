use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::{batch_provider_create, batch_provider_read, xai_identity};
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_provider_create::transports::BatchCreateTransport;
use betterlite_core::api::batch_provider_create::{BatchProviderCreate, BatchProviderCreateInput};
use betterlite_core::api::batch_provider_read::{
    BatchProviderRead, BatchProviderReadInput, BatchReadDispatch,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::batch_records::WireDecodeLimits;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use tokio::task::JoinSet;

use super::convert::{
    BatchAnswer, CallMode, SessionWire, create_hidden, create_input, create_settings, parse,
    read_hidden, read_input, read_settings, xai_settings,
};
use super::error::{BatchError, DispatchFailure, SessionError, batch_error};
use super::transport::{BatchDispatch, BatchTransport};

const TASK_WITHOUT_RESULT: &str = "the batch create task ended without a result";

// Everything one Python batch session owns: the environment snapshot, the section settings it resolved
// once, and the pooled client every provider call goes through.
pub(crate) struct BatchCore {
    client: ProxiedClient,
    environment: BTreeMap<String, String>,
    create_profile: batch_provider_create::Settings,
    read_profile: batch_provider_read::Settings,
    xai_profile: xai_identity::Settings,
    limits: ApiLimits,
    decode_limits: WireDecodeLimits,
}

impl BatchCore {
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire = parse(text)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment)
            .map_err(|source| SessionError::Proxy { source })?;
        let client = ProxiedClient::outbound(&Arc::new(policy))
            .map_err(|source| SessionError::Client { source })?;
        let module = wire.settings;
        let limits = ApiLimits::default();
        Ok(Self {
            create_profile: create_settings(&module, &environment)?,
            read_profile: read_settings(&module, &environment)?,
            xai_profile: xai_settings(&module, &environment)?,
            decode_limits: WireDecodeLimits {
                max_json_bytes: limits.max_json_bytes,
                max_event_bytes: limits.max_event_bytes,
                max_items: limits.max_items,
            },
            limits,
            environment,
            client,
        })
    }

    pub(crate) fn create_with<Transport: BatchCreateTransport>(
        &self,
        input: BatchProviderCreateInput,
        mode: CallMode,
        transport: Transport,
    ) -> Result<String, BatchError> {
        let prepared = create_input(input, mode);
        let catalog = BundledCatalog::default().get();
        let names = BTreeSet::new();
        let context = ResolutionContext {
            catalog: &catalog,
            known_model_names: &names,
            custom_providers: &names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        let service = BatchProviderCreate {
            settings: self.create_profile.clone(),
            resolution: &context,
            transport,
            limits: self.limits,
        };
        let result = service.create(&prepared)?;
        serde_json::to_string(&BatchAnswer {
            batch: &result.batch,
            hidden_params: create_hidden(&prepared, &result.call),
        })
        .map_err(|source| BatchError::Encode { source })
    }

    pub(crate) async fn retrieve_with<Dispatch>(
        &self,
        input: BatchProviderReadInput,
        mode: CallMode,
        dispatch: Dispatch,
    ) -> Result<String, BatchError>
    where
        Dispatch: BatchReadDispatch<Error = DispatchFailure>,
    {
        let prepared = read_input(input, mode);
        let service = BatchProviderRead {
            dispatch,
            limits: self.limits,
            settings: self.read_profile.clone(),
        };
        let result = service.run(&prepared).await?;
        serde_json::to_string(&BatchAnswer {
            batch: &result.batch,
            hidden_params: read_hidden(&prepared, &result.call),
        })
        .map_err(|source| BatchError::Encode { source })
    }
}

// main.py's create_batch, retrieve_batch and the awaitable twins, one session owning the client.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeBatchSession {
    core: Arc<BatchCore>,
}

#[pymethods]
impl NativeBatchSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let core = BatchCore::build(settings)?;
        Ok(Self {
            core: Arc::new(core),
        })
    }

    // the synchronous create: one call on the runtime, blocking this thread with the GIL released
    fn create(&self, py: Python<'_>, payload: &str, _blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let input: BatchProviderCreateInput = parse(payload)?;
        let provider = input.custom_llm_provider.as_str().to_owned();
        let model = input.model.clone();
        let core = Arc::clone(&self.core);
        let transport = BatchTransport {
            client: self.core.client.clone(),
            xai: self.core.xai_profile.clone(),
            limits: self.core.decode_limits,
        };
        py.detach(move || core.create_with(input, CallMode::Sync, transport))
            .map_err(|error| batch_error(&error, &provider, model.as_deref()))
    }

    // the synchronous retrieve: the dispatch is already awaitable
    fn retrieve(&self, py: Python<'_>, payload: &str, _blobs: Vec<Vec<u8>>) -> PyResult<String> {
        let input: BatchProviderReadInput = parse(payload)?;
        let provider = input.provider.name().to_owned();
        let model = input.model.clone();
        let core = Arc::clone(&self.core);
        let dispatch = BatchDispatch {
            client: self.core.client.clone(),
            xai: self.core.xai_profile.clone(),
            limits: self.core.decode_limits,
        };
        py.detach(move || {
            get_runtime().block_on(core.retrieve_with(input, CallMode::Sync, dispatch))
        })
        .map_err(|error| batch_error(&error, &provider, model.as_deref()))
    }

    // the awaitable create: the synchronous transport runs on a blocking task, not on the event loop
    fn acreate<'py>(
        &self,
        py: Python<'py>,
        payload: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let input: BatchProviderCreateInput = parse(payload)?;
        let provider = input.custom_llm_provider.as_str().to_owned();
        let model = input.model.clone();
        let core = Arc::clone(&self.core);
        let transport = BatchTransport {
            client: self.core.client.clone(),
            xai: self.core.xai_profile.clone(),
            limits: self.core.decode_limits,
        };
        future_into_py(py, async move {
            let mut tasks = JoinSet::new();
            tasks.spawn_blocking(move || core.create_with(input, CallMode::Async, transport));
            match tasks.join_next().await {
                Some(Ok(answer)) => {
                    answer.map_err(|error| batch_error(&error, &provider, model.as_deref()))
                }
                Some(Err(join_error)) => Err(batch_error(
                    &BatchError::Task(join_error.to_string()),
                    &provider,
                    model.as_deref(),
                )),
                None => Err(PyRuntimeError::new_err(TASK_WITHOUT_RESULT)),
            }
        })
    }

    // the awaitable retrieve: the dispatch is awaited on the event loop's runtime
    fn aretrieve<'py>(
        &self,
        py: Python<'py>,
        payload: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let input: BatchProviderReadInput = parse(payload)?;
        let provider = input.provider.name().to_owned();
        let model = input.model.clone();
        let core = Arc::clone(&self.core);
        let dispatch = BatchDispatch {
            client: self.core.client.clone(),
            xai: self.core.xai_profile.clone(),
            limits: self.core.decode_limits,
        };
        future_into_py(py, async move {
            core.retrieve_with(input, CallMode::Async, dispatch)
                .await
                .map_err(|error| batch_error(&error, &provider, model.as_deref()))
        })
    }
}
