use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::api::realtime_http_calls::{
    RealtimeHttpCallSettings, load_settings as load_http_settings,
};
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
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::Utc;
use indexmap::IndexMap;
use serde::Deserialize;
use uuid::Uuid;

use crate::completion::setup::outbound_client;

use super::error::SessionError;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SessionWire {
    environment: Option<BTreeMap<String, String>>,
    litellm_settings: Option<Json>,
}

// what every call of one session shares: the process environment, the loaded HTTP settings and the pooled client
pub(crate) struct RealtimeSession {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) http_settings: RealtimeHttpCallSettings,
    pub(crate) catalog: Arc<ModelCatalog>,
    pub(crate) known_model_names: BTreeSet<String>,
    pub(crate) custom_providers: BTreeSet<String>,
    pub(crate) registry: JSONProviderRegistry,
    pub(crate) client: ProxiedClient,
    pub(crate) redactor: Redactor,
    pub(crate) sink: SilentSink,
}

impl RealtimeSession {
    pub(crate) fn build(settings: &str) -> Result<Self, SessionError> {
        let wire: SessionWire = serde_json::from_str(settings)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let document = serde_json::to_string(&BTreeMap::from([(
            "litellm_settings",
            wire.litellm_settings
                .unwrap_or(Json::Object(IndexMap::new())),
        )]))?;
        let http_settings = load_http_settings(&document, environment.clone())
            .map_err(SessionError::HttpSettings)?;
        let client = outbound_client(&environment)
            .map_err(|error| SessionError::Client(error.to_string()))?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SessionError::Redact)?;
        Ok(Self {
            catalog: BundledCatalog::default().get(),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            registry: JSONProviderRegistry::load(),
            client,
            environment,
            http_settings,
            redactor,
            sink: SilentSink,
        })
    }

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

    // B247's logging state for one call; the Python package registers no callbacks to hand records to
    pub(crate) fn logging(&self, call_type: &str) -> Result<LoggingState, SessionError> {
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
            &self.redactor,
            || Uuid::new_v4().to_string(),
            Duration::default,
        )
        .map_err(SessionError::Logging)
    }

    pub(crate) fn pre_post(&self) -> PrePostEnv<'_> {
        PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: Utc::now().fixed_offset(),
        }
    }
}

pub(crate) struct SilentSink;

impl LogSink for SilentSink {
    fn emit(&self, _record: LogRecord) {}
}
