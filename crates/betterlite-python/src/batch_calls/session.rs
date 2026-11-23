use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
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
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use serde::de::DeserializeOwned;
use tokio::task::JoinSet;

use super::convert::{
    BatchAnswer, CallMode, FinishedCall, HiddenParams, SessionWire, create_input, parse,
    read_input, section_settings,
};
use super::error::{BatchError, DispatchFailure, SessionError, batch_error};
use super::transport::{BatchDispatch, BatchTransport};

const TASK_WITHOUT_RESULT: &str = "the batch create task ended without a result";

// One Python batch session owns its environment snapshot, the section settings resolved once, and the pooled client.
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
            create_profile: section_settings::<batch_provider_create::Raw>(&module, &environment)?,
            read_profile: section_settings::<batch_provider_read::Raw>(&module, &environment)?,
            xai_profile: section_settings::<xai_identity::Raw>(&module, &environment)?,
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

    fn transport(&self) -> BatchTransport {
        BatchTransport {
            client: self.client.clone(),
            xai: self.xai_profile.clone(),
            limits: self.decode_limits,
        }
    }

    fn dispatch(&self) -> BatchDispatch {
        BatchDispatch {
            client: self.client.clone(),
            xai: self.xai_profile.clone(),
            limits: self.decode_limits,
        }
    }

    pub(crate) fn create_with<Transport>(
        &self,
        input: BatchProviderCreateInput,
        mode: CallMode,
        transport: Transport,
    ) -> Result<String, BatchError>
    where
        Transport: BatchCreateTransport,
    {
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
        let answer = BatchAnswer {
            batch: &result.batch,
            hidden_params: HiddenParams::new(
                prepared.custom_llm_provider.as_str(),
                prepared.model.as_deref(),
                FinishedCall::Create(&result.call),
            ),
        };
        serde_json::to_string(&answer).map_err(|source| BatchError::Encode { source })
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
        let answer = BatchAnswer {
            batch: &result.batch,
            hidden_params: HiddenParams::new(
                prepared.provider.name(),
                prepared.model.as_deref(),
                FinishedCall::Read(&result.call),
            ),
        };
        serde_json::to_string(&answer).map_err(|source| BatchError::Encode { source })
    }
}

// the file contents a batch payload carries, one blob per file it named
type Blobs = Vec<Vec<u8>>;

// One batch payload a session runs: how it parses, the provider and model its failure names, and the call itself.
trait BatchOperation {
    type Input: DeserializeOwned + Send + 'static;

    fn owner(input: &Self::Input) -> (String, Option<String>);

    fn run(
        core: Arc<BatchCore>,
        input: Self::Input,
        mode: CallMode,
    ) -> impl Future<Output = Result<String, BatchError>> + Send;
}

struct CreateCall;

struct ReadCall;

impl BatchOperation for CreateCall {
    type Input = BatchProviderCreateInput;

    fn owner(input: &Self::Input) -> (String, Option<String>) {
        let provider = input.custom_llm_provider.as_str().to_owned();
        (provider, input.model.clone())
    }

    fn run(
        core: Arc<BatchCore>,
        input: Self::Input,
        mode: CallMode,
    ) -> impl Future<Output = Result<String, BatchError>> + Send {
        async move {
            let transport = core.transport();
            let mut tasks = JoinSet::new();
            tasks.spawn_blocking(move || core.create_with(input, mode, transport));
            match tasks.join_next().await {
                Some(Ok(answer)) => answer,
                Some(Err(join_error)) => Err(BatchError::Task(join_error.to_string())),
                None => Err(BatchError::Task(TASK_WITHOUT_RESULT.to_owned())),
            }
        }
    }
}

impl BatchOperation for ReadCall {
    type Input = BatchProviderReadInput;

    fn owner(input: &Self::Input) -> (String, Option<String>) {
        let provider = input.provider.name().to_owned();
        (provider, input.model.clone())
    }

    fn run(
        core: Arc<BatchCore>,
        input: Self::Input,
        mode: CallMode,
    ) -> impl Future<Output = Result<String, BatchError>> + Send {
        async move {
            let dispatch = core.dispatch();
            core.retrieve_with(input, mode, dispatch).await
        }
    }
}

// the one answer every batch method waits on, the failure naming the provider and model the request asked for
async fn answer_text<Operation>(
    core: Arc<BatchCore>,
    input: Operation::Input,
    mode: CallMode,
) -> PyResult<String>
where
    Operation: BatchOperation,
{
    let (provider, model) = Operation::owner(&input);
    Operation::run(core, input, mode)
        .await
        .map_err(|error| batch_error(&error, &provider, model.as_deref()))
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

    // the synchronous create: a blocking task on the runtime, this thread waiting with the GIL released
    fn create(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> PyResult<String> {
        let input: BatchProviderCreateInput = parse(payload)?;
        let core = Arc::clone(&self.core);
        py.detach(move || {
            get_runtime().block_on(answer_text::<CreateCall>(core, input, CallMode::Sync))
        })
    }

    // the synchronous retrieve: the dispatch is already awaitable, so the runtime drives it here
    fn retrieve(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> PyResult<String> {
        let input: BatchProviderReadInput = parse(payload)?;
        let core = Arc::clone(&self.core);
        py.detach(move || {
            get_runtime().block_on(answer_text::<ReadCall>(core, input, CallMode::Sync))
        })
    }

    // the awaitable create: the synchronous transport runs on a blocking task, not on the event loop
    fn acreate(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> PyResult<Py<PyAny>> {
        let input: BatchProviderCreateInput = parse(payload)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, answer_text::<CreateCall>(core, input, CallMode::Async))
            .map(Bound::unbind)
    }

    // the awaitable retrieve: the dispatch is awaited on the event loop's runtime
    fn aretrieve(&self, py: Python<'_>, payload: &str, _blobs: Blobs) -> PyResult<Py<PyAny>> {
        let input: BatchProviderReadInput = parse(payload)?;
        let core = Arc::clone(&self.core);
        future_into_py(py, answer_text::<ReadCall>(core, input, CallMode::Async)).map(Bound::unbind)
    }
}
