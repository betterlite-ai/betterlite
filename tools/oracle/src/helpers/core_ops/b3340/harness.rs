use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use betterlite_core::api::video_remix_list_transport::VideoRemixListTransport;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};

use super::client::{Source, Wire};
use super::config::Lookup;
use super::failed;

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The logging environment the pre-call records into, with a clock no case compares
pub(crate) struct LogEnv {
    pub(crate) redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
}

impl LogEnv {
    fn new() -> Result<Self, Box<HelperError>> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| failed(&error))?;
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .map_err(|error| failed(&error))?;
        Ok(Self {
            redactor,
            sink: Sink,
            now,
        })
    }
}

// The pre-call environment every case shares: no debug capture, no callbacks, the case's clock
pub(crate) fn base_env(env: &LogEnv) -> PrePostEnv<'_> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &env.redactor,
        log: &env.sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: env.now,
    }
}

// The resolution fixture avideo_list infers from an empty model with
pub(crate) struct Resolver {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Resolver {
    fn new() -> Self {
        Self {
            catalog: ModelCatalog::default(),
            names: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    pub(crate) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

// One case's logging object, config, pool and transport, all sharing one wire
pub(crate) struct Bench {
    pub(crate) state: LoggingState,
    pub(crate) env: LogEnv,
    pub(crate) configs: Lookup,
    pub(crate) transport: VideoRemixListTransport<Source>,
    pub(crate) resolver: Resolver,
    pub(crate) source: Source,
}

impl Bench {
    pub(crate) fn new(status: u16, body: &str, call_id: &str) -> Result<Self, Box<HelperError>> {
        let env = LogEnv::new()?;
        let state = logging_state(call_id, &env.redactor)?;
        let source = Source {
            wire: Wire::replying(status, body),
        };
        Ok(Self {
            state,
            env,
            configs: Lookup::new(),
            transport: VideoRemixListTransport {
                source: source.clone(),
            },
            resolver: Resolver::new(),
            source,
        })
    }
}

// LiteLLM's Logging(model="", messages=None, stream=False, call_type="", litellm_call_id=caller, function_id="")
fn logging_state(call_id: &str, redactor: &Redactor) -> Result<LoggingState, Box<HelperError>> {
    let init: LoggingInit = serde_json::from_value(serde_json::json!({
        "model": "",
        "call_type": "",
        "start_time": "2026-01-01T00:00:00+00:00",
        "litellm_call_id": call_id,
        "function_id": "",
        "litellm_trace_id": "trace-1",
    }))
    .map_err(|error| failed(&error))?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .map_err(|error| failed(&error))
}
