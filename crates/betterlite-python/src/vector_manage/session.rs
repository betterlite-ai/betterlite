use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use betterlite_config::api::vector_store_read::loads_read_timeout;
use betterlite_config::api::vector_store_write::loads_request_timeout;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::openai_credentials::{
    Raw as OpenAiCredentialsRaw, Settings as OpenAiCredentialsSettings,
};
use betterlite_core::api::vector_store_read::{
    TransportRequest as ReadTransport, VectorStoreListRequest, VectorStoreReadDeps,
    VectorStoreReadRequest,
};
use betterlite_core::api::vector_store_write::service::Operation as WriteOperation;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState, LoggingStateError};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_wrapper::ports::{IdSource, WrapperClock};
use chrono::{DateTime, FixedOffset, Utc};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use uuid::Uuid;

use crate::completion::setup::openai_defaults;

use self::operations::{ReadRequest, read, write};
use super::convert::{ListWire, RetrieveWire, WriteWire, params, parse, read_transport};
use super::error::{NativeFailure, setup};
use super::service::SessionService;
use super::source::ProxiedSource;

mod operations;
mod tests;

const DEFAULT_PROVIDER: &str = "openai";

// The Python-side settings snapshot: the module globals a vector store call reads, and an environment when a test pins one.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SessionWire {
    environment: Option<BTreeMap<String, String>>,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    openai_key: Option<SecretString>,
    organization: Option<String>,
}

// What one native call runs: each operation's wire, with the flag its method passes on
pub(crate) enum Pending {
    Retrieve(RetrieveWire),
    List(ListWire),
    Update(WriteWire),
    Delete(WriteWire),
}

impl Pending {
    // the call_type the SDK's @client wrapper names after the function it decorates
    const fn call_type(&self, asynchronous: bool) -> &'static str {
        match (self, asynchronous) {
            (Self::Retrieve(_), false) => "retrieve",
            (Self::Retrieve(_), true) => "aretrieve",
            (Self::List(_), false) => "list",
            (Self::List(_), true) => "alist",
            (Self::Update(_), false) => "update",
            (Self::Update(_), true) => "aupdate",
            (Self::Delete(_), false) => "delete",
            (Self::Delete(_), true) => "adelete",
        }
    }
}

pub(crate) struct SessionCore {
    credentials: OpenAiCredentialsSettings,
    globals: LiteLlmGlobals,
    env_api_key: Option<SecretString>,
    source: ProxiedSource,
    catalog: Arc<ModelCatalog>,
    known_models: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    read_timeout: f64,
    write_timeout: f64,
    redactor: Redactor,
    clock: Arc<dyn WrapperClock>,
    ids: Arc<dyn IdSource>,
}

impl SessionCore {
    // The clock and the id source are fields so a test pins every time and id the session writes
    #[expect(
        clippy::too_many_lines,
        reason = "one flat list of every setting a session reads, with no branches"
    )]
    pub(crate) fn build(
        text: &str,
        clock: Arc<dyn WrapperClock>,
        ids: Arc<dyn IdSource>,
    ) -> Result<Self, NativeFailure> {
        let wire: SessionWire = parse(text, 0)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let config = ConfigLoader {
            lookup: environment.clone(),
        }
        .load("{}")
        .map_err(|source| setup("the configuration could not be loaded", &source))?;
        let credentials = OpenAiCredentialsRaw::default()
            .convert(&environment)
            .map_err(|source| setup("the credentials could not be read", &source))?;
        let read_timeout = loads_read_timeout(&environment)
            .map_err(|source| setup("the read timeout could not be read", &source))?;
        let write_timeout = loads_request_timeout(&environment)
            .map_err(|source| setup("the write timeout could not be read", &source))?;
        let routing = Arc::new(
            ProxyPolicy::resolve(&config.litellm_settings.proxy_trust, &environment)
                .map_err(|source| setup("the proxy environment could not be read", &source))?,
        );
        let env_api_key = openai_defaults(&environment).api_keys.first().cloned();
        let globals = LiteLlmGlobals {
            api_base: wire.api_base,
            api_key: wire.api_key,
            openai_key: wire.openai_key,
            organization: wire.organization,
        };
        Ok(Self {
            credentials,
            globals,
            env_api_key,
            source: ProxiedSource::new(config, environment.clone(), routing),
            catalog: BundledCatalog::default().get(),
            known_models: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment,
            read_timeout,
            write_timeout,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
                .map_err(|source| setup("the redactor could not be built", &source))?,
            clock,
            ids,
        })
    }

    // One logging state for this call, and the read or write core's service behind the session's clients
    pub(crate) async fn run(&self, pending: Pending, flag: bool) -> Result<Json, NativeFailure> {
        let state = self
            .logging_state(pending.call_type(flag))
            .map_err(|error| NativeFailure::Setup {
                message: format!("the vector store call could not be logged: {error}"),
            })?;
        let logging = Mutex::new(state);
        match pending {
            Pending::Retrieve(wire) => {
                let parameters = params(&wire.transport.kwargs)?;
                let request = ReadRequest::Retrieve(VectorStoreReadRequest {
                    transport: read_transport!(wire, &parameters),
                    vector_store_id: &wire.vector_store_id,
                });
                read(self, request, flag, &logging).await
            }
            Pending::List(wire) => {
                let parameters = params(&wire.transport.kwargs)?;
                let request = ReadRequest::List(VectorStoreListRequest {
                    transport: read_transport!(wire, &parameters),
                    after: wire.after.as_deref(),
                    before: wire.before.as_deref(),
                    limit: Some(wire.limit),
                    order: Some(wire.order.as_deref()),
                });
                read(self, request, flag, &logging).await
            }
            Pending::Update(wire) => {
                write(self, WriteOperation::Update, &wire, flag, &logging).await
            }
            Pending::Delete(wire) => {
                write(self, WriteOperation::Delete, &wire, flag, &logging).await
            }
        }
    }

    fn service(&self) -> SessionService<'_> {
        SessionService {
            credentials: &self.credentials,
            globals: &self.globals,
            env_api_key: &self.env_api_key,
            source: &self.source,
            redactor: &self.redactor,
            clock: self.clock.as_ref(),
        }
    }

    fn read_deps<'deps>(
        &'deps self,
        adapter: &'deps SessionService<'deps>,
        logging: &'deps Mutex<LoggingState>,
    ) -> VectorStoreReadDeps<'deps, ProxiedClient> {
        VectorStoreReadDeps {
            configs: adapter,
            logging,
            redactor: &self.redactor,
            supplied: None,
            request_timeout_seconds: self.read_timeout,
        }
    }

    // B174's provider resolution reads the catalog, the configured names and the environment
    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: self.catalog.as_ref(),
            known_model_names: &self.known_models,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    // The Pre Call logging a Python session starts from where the SDK's decorator would have built it
    fn logging_state(&self, call_type: &str) -> Result<LoggingState, LoggingStateError> {
        let init = LoggingInit {
            model: String::new(),
            call_type: call_type.to_owned(),
            start_time: self.clock.now(),
            litellm_call_id: self.ids.next_call_id(),
            function_id: String::new(),
            messages: None,
            stream: None,
            litellm_trace_id: None,
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        };
        LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &self.redactor,
            || self.ids.next_call_id(),
            || self.clock.monotonic(),
        )
    }
}

// LiteLLM's call wrapper supplies the clock and uuid4; completion::ports has the same pair but is private
pub(crate) struct SystemClock {
    started: Instant,
}

impl SystemClock {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl WrapperClock for SystemClock {
    fn now(&self) -> DateTime<FixedOffset> {
        Utc::now().fixed_offset()
    }

    fn monotonic(&self) -> Duration {
        self.started.elapsed()
    }
}

pub(crate) struct SystemIds;

impl IdSource for SystemIds {
    fn next_call_id(&self) -> String {
        Uuid::new_v4().to_string()
    }
}
