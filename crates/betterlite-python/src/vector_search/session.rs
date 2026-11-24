use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use betterlite_config::api::vector_store_search::Settings;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::openai_credentials::Raw as OpenAiCredentialsRaw;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_core::api::vector_store_search::error::VectorStoreSearchError;
use betterlite_core::api::vector_store_search::service::SdkDeps;
use betterlite_core::api::vector_store_search::service::create_call::create as create_store;
use betterlite_core::api::vector_store_search::service::search_call::search as search_store;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState, LoggingStateError};
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_wrapper::ports::{IdSource, WrapperClock};
use chrono::{DateTime, FixedOffset, Utc};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use uuid::Uuid;

use crate::completion::setup::openai_defaults;

use super::convert::{CreateWire, SearchWire, create_call, parse, search_call};
use super::error::SessionError;
use super::seams::{Configs, Executors, Filters, Inline};
use super::source::{ProxiedSource, SilentSink};

mod tests;

const CREATE_CALL_TYPE: &str = "vector_store_create";
const SEARCH_CALL_TYPE: &str = "vector_store_search";

// The Python-side settings snapshot: the module globals a vector store call reads, and an environment when a test pins one.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SessionWire {
    pub(crate) environment: Option<BTreeMap<String, String>>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    pub(crate) openai_key: Option<SecretString>,
    pub(crate) organization: Option<String>,
}

// What one native call runs: the create or the search, parsed and ready for the service behind the session
pub(crate) enum Pending {
    Create(CreateWire),
    Search(SearchWire),
}

// Everything one Python session owns: the environment snapshot, the configs it resolves and the client its requests use.
pub(crate) struct SessionCore {
    configs: Configs,
    filters: Filters,
    executors: Executors,
    signing: Inline,
    source: ProxiedSource,
    settings: Settings,
    pub(crate) redactor: Redactor,
    clock: Arc<dyn WrapperClock>,
    ids: Arc<dyn IdSource>,
}

impl SessionCore {
    // The clock and the id source are fields so a test pins every time and id the session writes
    pub(crate) fn build(
        text: &str,
        clock: Arc<dyn WrapperClock>,
        ids: Arc<dyn IdSource>,
    ) -> Result<Self, SessionError> {
        let wire: SessionWire = parse(text, 0)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let config = ConfigLoader {
            lookup: environment.clone(),
        }
        .load("{}")
        .map_err(|source| SessionError::Config { source })?;
        let credentials = OpenAiCredentialsRaw::default()
            .convert(&environment)
            .map_err(|source| SessionError::Config { source })?;
        let settings =
            Settings::load(&environment).map_err(|source| SessionError::Settings { source })?;
        let routing = Arc::new(
            ProxyPolicy::resolve(&config.litellm_settings.proxy_trust, &environment)
                .map_err(|source| SessionError::Proxy { source })?,
        );
        let env_api_key = openai_defaults(&environment).api_keys.first().cloned();
        let globals = LiteLlmGlobals {
            api_base: wire.api_base,
            api_key: wire.api_key,
            openai_key: wire.openai_key,
            organization: wire.organization,
        };
        Ok(Self {
            configs: Configs::new(credentials, globals, env_api_key),
            filters: Filters,
            executors: Executors,
            signing: Inline,
            source: ProxiedSource::new(config, environment, routing),
            settings,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
                .map_err(|source| SessionError::Redactor { source })?,
            clock,
            ids,
        })
    }

    // create and search: one logging state, one HTTP client and B1064's or B1058's service behind them
    pub(crate) async fn run(&self, pending: Pending) -> Result<Json, VectorStoreSearchError> {
        let sink = SilentSink;
        let env = self.pre_post(&sink);
        let call_type = match &pending {
            Pending::Create(_) => CREATE_CALL_TYPE,
            Pending::Search(_) => SEARCH_CALL_TYPE,
        };
        let mut state = self
            .logging_state(call_type)
            .map_err(VectorStoreSearchError::from)?;
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        let deps = self.deps();
        match pending {
            Pending::Create(wire) => {
                let call = create_call(&wire);
                create_store(&deps, &call, &mut log).await
            }
            Pending::Search(wire) => {
                let call = search_call(&wire);
                search_store(&deps, &call, &mut log).await
            }
        }
    }

    fn deps(&self) -> SdkDeps<'_, ProxiedSource> {
        SdkDeps {
            configs: &self.configs,
            filters: &self.filters,
            executors: &self.executors,
            signing: &self.signing,
            registry: None,
            source: &self.source,
            supplied: None,
            settings: self.settings,
        }
    }

    fn pre_post<'env>(&'env self, sink: &'env SilentSink) -> PrePostEnv<'env> {
        PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.clock.now(),
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
