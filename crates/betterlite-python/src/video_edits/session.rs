use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_core::api::video_edit_transport::config::VideoLogging;
use betterlite_core::api::video_edit_transport::error::VideoClientError;
use betterlite_core::api::video_edit_transport::seams::{
    VideoClientSource, VideoLoggingHandle, VideoPreCall,
};
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::pool::PoolError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::Utc;
use indexmap::IndexMap;
use serde::Deserialize;
use uuid::Uuid;

use super::adapter::OpenAiEditConfig;
use super::convert::{ConvertError, SettingsWire};
use super::error::SessionError;
use crate::completion::setup::{outbound_client, runtime_config};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SessionWire {
    environment: Option<BTreeMap<String, String>>,
}

// what every call of one session shares: the process environment, its TLS policy and the pooled client
pub(crate) struct EditSession {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) client: ProxiedClient,
    tls: TlsPolicy,
    pub(crate) request_timeout_seconds: f64,
    pub(crate) redactor: Redactor,
}

impl EditSession {
    pub(crate) fn build(settings: &str) -> Result<Self, SessionError> {
        let wire: SessionWire =
            serde_json::from_str(settings).map_err(|source| ConvertError::Json { source })?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let tls = TlsPolicy::resolve(&runtime_config(), &environment, None)?;
        let client = outbound_client(&environment)?;
        let request_timeout = RequestTimeout::resolve(&environment)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
        Ok(Self {
            client,
            request_timeout_seconds: request_timeout.seconds,
            environment,
            tls,
            redactor,
        })
    }

    // ast-grep-ignore: no-one-line-functions - one client table per call from the TLS policy this module keeps private
    pub(crate) fn clients(&self) -> HttpClients {
        HttpClients::new(self.tls.clone(), Arc::clone(&self.client.policy))
    }
}

pub(crate) fn config(
    session: &EditSession,
    settings: &SettingsWire,
    video_name: Option<String>,
) -> OpenAiEditConfig {
    OpenAiEditConfig {
        environment: session.environment.clone(),
        global_api_key: settings.api_key.clone(),
        openai_key: settings.openai_key.clone(),
        video_name,
    }
}

// B247's logging state for one call; the Python package registers no callbacks to hand records to
pub(crate) fn logging(redactor: &Redactor, call_type: &str) -> Result<LoggingState, SessionError> {
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
        Duration::default,
    )
    .map_err(SessionError::Logging)
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

// the edit and extension transports' client factories over the one pooled client
pub(crate) struct EditSource {
    environment: BTreeMap<String, String>,
    policy: Arc<ProxyPolicy>,
    default: ProxiedClient,
}

impl EditSource {
    pub(crate) fn new(session: &EditSession) -> Self {
        Self {
            environment: session.environment.clone(),
            policy: Arc::clone(&session.client.policy),
            default: session.client.clone(),
        }
    }

    fn client_with(&self, ssl_verify: Option<bool>) -> Result<ProxiedClient, VideoClientError> {
        match ssl_verify {
            Some(false) => {
                let tls = TlsPolicy {
                    verification: Verification::Disabled,
                    ca_file: None,
                    certificate: None,
                    http2: false,
                    ecdh_curve: None,
                    user_agent: user_agent(&self.environment),
                };
                ProxiedClient::outbound_with_tls(&self.policy, &tls)
                    .map_err(|source| VideoClientError::Pool(PoolError::Client(source)))
            }
            Some(true) | None => Ok(self.default.clone()),
        }
    }
}

// TlsPolicy's default agent, read the way every family reads LITELLM_USER_AGENT
fn user_agent(environment: &BTreeMap<String, String>) -> String {
    environment
        .get("LITELLM_USER_AGENT")
        .cloned()
        .unwrap_or_else(|| format!("betterlite/{}", env!("CARGO_PKG_VERSION")))
}

impl VideoClientSource for EditSource {
    type Client = ProxiedClient;

    fn sync_client(&self, ssl_verify: Option<bool>) -> Result<Self::Client, VideoClientError> {
        self.client_with(ssl_verify)
    }

    fn async_client(
        &self,
        _provider: &str,
        ssl_verify: Option<bool>,
    ) -> Result<Self::Client, VideoClientError> {
        self.client_with(ssl_verify)
    }
}

// B249's logging handle with no sink mounted: pre_call has nothing to write and the transforms get an empty object
pub(crate) struct SilentLog;

impl VideoLoggingHandle for SilentLog {
    fn pre_call(&self, _call: &VideoPreCall<'_>) -> Result<(), PrePostError> {
        Ok(())
    }

    fn logging_object(&self) -> VideoLogging {
        VideoLogging::default()
    }
}
