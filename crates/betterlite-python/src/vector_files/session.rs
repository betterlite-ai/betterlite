use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::api::vector_store_write::loads_request_timeout;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::openai_credentials::Raw as OpenAiCredentialsRaw;
use betterlite_env::EnvError;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState, LoggingStateError};
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::Utc;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use uuid::Uuid;

use crate::completion::setup::openai_defaults;

use super::convert::{ConvertError, CreateWire, ListWire, ReadWire, WriteWire, parse};
use super::service::Configs;
use super::source::{ProxiedSource, SilentSink};

// The Python-side settings snapshot: the module globals a vector store file call reads, and an environment when a test pins one.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SessionWire {
    pub(crate) environment: Option<BTreeMap<String, String>>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    pub(crate) openai_key: Option<SecretString>,
    pub(crate) organization: Option<String>,
}

// What one native call runs: the operation parsed and ready for the service behind the session
pub(crate) enum PendingOperation {
    Create(CreateWire),
    List(ListWire),
    Retrieve(ReadWire),
    RetrieveContent(ReadWire),
    Update(WriteWire),
    Delete(WriteWire),
}

// One native call: what it runs, and whether the async handler answers it
pub(crate) struct Pending {
    pub(crate) operation: PendingOperation,
    pub(crate) asynchronous: bool,
}

// Everything one Python session owns: the environment snapshot, the configs it resolves and the client its requests use.
pub(crate) struct SessionCore {
    pub(crate) configs: Configs,
    pub(crate) source: ProxiedSource,
    pub(crate) redactor: Redactor,
    pub(crate) request_timeout_seconds: f64,
}

impl SessionCore {
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
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
        let request_timeout_seconds = loads_request_timeout(&environment)
            .map_err(|source| SessionError::Config { source })?;
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
            source: ProxiedSource::new(config, environment, routing),
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
                .map_err(|source| SessionError::Redactor { source })?,
            request_timeout_seconds,
        })
    }

    // The Pre Call logging a Python session starts from where the SDK's decorator would have built it
    pub(crate) fn logging_state(&self, call_type: &str) -> Result<LoggingState, LoggingStateError> {
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
            || Duration::ZERO,
        )
    }

    // The environment a handler's pre-call logs through; a Python session has no callbacks, sentry or logger function
    pub(crate) fn pre_post<'env>(&'env self, sink: &'env SilentSink) -> PrePostEnv<'env> {
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
            now: Utc::now().fixed_offset(),
        }
    }
}

// Everything that can fail while a session is built, before any request is made
#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error(transparent)]
    Json(#[from] ConvertError),
    #[error("the session configuration could not be loaded: {source}")]
    Config {
        #[source]
        source: ConfigError,
    },
    #[error("the proxy environment could not be read: {source}")]
    Proxy {
        #[source]
        source: EnvError,
    },
    #[error("the credential redactor could not be built: {source}")]
    Redactor {
        #[source]
        source: RedactionError,
    },
}
