use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::loader::startup::process_environment;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::Utc;
use indexmap::IndexMap;
use serde::Deserialize;
use uuid::Uuid;

use super::adapter::OpenAiVideoConfig;
use super::convert::{ConvertError, Reference, SettingsWire};
use super::error::{RunError, SessionError};
use crate::completion::setup::outbound_client;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SessionWire {
    environment: Option<BTreeMap<String, String>>,
}

// what every call of one session shares: the process environment, the catalog resolution reads and the pooled client
pub(crate) struct VideoSession {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) catalog: Arc<ModelCatalog>,
    pub(crate) known_model_names: BTreeSet<String>,
    pub(crate) custom_providers: BTreeSet<String>,
    pub(crate) registry: JSONProviderRegistry,
    pub(crate) client: ProxiedClient,
    pub(crate) request_timeout_seconds: f64,
    // the redactor and the sink the remix and list calls write through, owned here so a call site can lend them out
    pub(crate) redactor: Redactor,
    pub(crate) sink: SilentSink,
}

impl VideoSession {
    pub(crate) fn build(settings: &str) -> Result<Self, SessionError> {
        let wire: SessionWire =
            serde_json::from_str(settings).map_err(|source| ConvertError::Json { source })?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let client = outbound_client(&environment)?;
        let request_timeout = RequestTimeout::resolve(&environment)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
        Ok(Self {
            catalog: BundledCatalog::default().get(),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            registry: JSONProviderRegistry::load(),
            client,
            request_timeout_seconds: request_timeout.seconds,
            environment,
            redactor,
            sink: SilentSink,
        })
    }

    // get_llm_provider's view of this session, with no module-level credentials to add
    pub(crate) fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

pub(crate) fn config(
    session: &VideoSession,
    settings: &SettingsWire,
    reference: Option<Reference>,
) -> OpenAiVideoConfig {
    OpenAiVideoConfig {
        environment: session.environment.clone(),
        global_api_key: settings.api_key.clone(),
        openai_key: settings.openai_key.clone(),
        reference,
    }
}

// B247's logging state for one call; the Python package registers no callbacks to hand records to
pub(crate) fn logging(redactor: &Redactor, call_type: &str) -> Result<LoggingState, RunError> {
    let init = LoggingInit {
        model: String::new(),
        call_type: call_type.to_owned(),
        start_time: Utc::now().fixed_offset(),
        litellm_call_id: Uuid::new_v4().to_string(),
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
    let mut correlation = CorrelationContext::default();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut correlation,
            settings: CorrelationSettings::default(),
        },
        redactor,
        || Uuid::new_v4().to_string(),
        Duration::default(),
    )
    .map_err(|source| RunError::Session(SessionError::Logging(source)))
}

pub(crate) struct SilentSink;

impl LogSink for SilentSink {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) fn pre_post<'env>(sink: &'env SilentSink, redactor: &'env Redactor) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: Utc::now().fixed_offset(),
    }
}
