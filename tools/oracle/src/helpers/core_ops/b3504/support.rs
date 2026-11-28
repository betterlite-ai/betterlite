pub(crate) mod server;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::studio_identity::Settings;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::agents::collection::{AgentCollection, AgentCollectionFailure};
use betterlite_http::interactions::agents_create_read::logging::{
    AgentLogging, AgentPostCall, AgentPreCall,
};
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_vertex::studio_agents::config::GeminiAgentsConfig;
use betterlite_vertex::studio_agents::config::error::GeminiAgentError;
use chrono::DateTime;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;

use server::Server;
use std::fmt::Display;

#[derive(Debug, Deserialize)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: Value,
}

pub(crate) struct Fixture {
    pub(crate) config: GeminiAgentsConfig,
    pub(crate) clients: HttpClients,
    pub(crate) logging: LoggingState,
    pub(crate) records: Recorder,
    pub(crate) redactor: Redactor,
}

impl Fixture {
    pub(crate) fn new(settings: Settings) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            config: GeminiAgentsConfig { settings },
            clients: HttpClients::new(policy(), proxy_policy()?),
            logging: logging_state()?,
            records: Recorder,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(helper_failure)?,
        })
    }

    pub(crate) fn collection(&mut self) -> AgentCollection<'_, GeminiAgentsConfig> {
        AgentCollection {
            config: &self.config,
            clients: &mut self.clients,
            logging: &mut self.logging,
            records: &mut self.records,
            redactor: &self.redactor,
            request_timeout: Duration::from_secs(600),
        }
    }
}

// the pipeline requires a logger, and the compared answer holds the wire bytes instead of its callbacks
pub(crate) struct Recorder;

impl AgentLogging for Recorder {
    fn pre_call(&mut self, _call: AgentPreCall<'_>) {}
    fn post_call(&mut self, _call: AgentPostCall<'_>) {}
}
pub(crate) fn logging_state() -> Result<LoggingState, Box<HelperError>> {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "fixture".to_owned(),
            call_type: "fixture".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
                .map_err(helper_failure)?,
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(helper_failure)?,
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .map_err(helper_failure)
}

fn policy() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-oracle/0".to_owned(),
    }
}

fn proxy_policy() -> Result<Arc<ProxyPolicy>, Box<HelperError>> {
    ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .map(Arc::new)
        .map_err(helper_failure)
}

pub(crate) fn helper_failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "AgentCollectionError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

// the class carries the status LiteLLM's raised exception reports, and the compared type, param and code stay null
pub(crate) fn raised(error: &AgentCollectionFailure<GeminiAgentError>) -> Box<HelperError> {
    Box::new(HelperError {
        class: "AgentCollectionError".to_owned(),
        status: Some(error.class.metadata().status_code),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

pub(crate) fn answer_value(
    server: &Server,
    extra: &BTreeSet<String>,
    result: &Value,
    logging: &LoggingState,
) -> Value {
    json!({
        "requests": server.records(extra),
        "result": result,
        "model": logging.model.clone(),
    })
}
