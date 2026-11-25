use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use betterlite_config::sections::openai_credentials::Settings;
use betterlite_core::api::container_file_endpoints::handler::{
    ContainerConfigLookup, ContainerIdDecoder, DecodedContainerCall,
};
use betterlite_core::containers::container_request_parameters::ContainerFieldTypeError;
// ast-grep-ignore: no-long-names
use betterlite_core::containers::container_request_parameters::decode_managed_container_id_for_request;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::container_contracts::config::BaseContainerConfig;
use betterlite_openai::containers::OpenAiContainerConfig;
use betterlite_openai::containers::container_file_http::PreCallHook;
use betterlite_openai::containers::costs::ContainerCosts;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_wrapper::ports::{IdSource, WrapperClock};
use chrono::{DateTime, FixedOffset, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::SecretString;
use uuid::Uuid;

use super::error::SessionError;

pub(crate) struct Configs {
    openai: OpenAiContainerConfig,
}

impl Configs {
    pub(crate) fn new(
        settings: Settings,
        globals: LiteLlmGlobals,
        env_api_key: Option<SecretString>,
        costs: Arc<dyn ContainerCosts>,
    ) -> Self {
        Self {
            openai: OpenAiContainerConfig {
                settings,
                globals,
                env_api_key,
                costs,
            },
        }
    }
}

impl ContainerConfigLookup for Configs {
    fn container_config(&self, provider: ProviderId) -> Option<&dyn BaseContainerConfig> {
        let config: &dyn BaseContainerConfig = &self.openai;
        (provider == ProviderId::Openai).then_some(config)
    }
}

// A container file call never bills; the config's cost source serves container create alone
#[derive(Debug)]
pub(crate) struct NoCosts;

impl ContainerCosts for NoCosts {
    // Decimal::ZERO is Decimal::default(), so no test can tell that mutant from this line
    #[cfg_attr(test, mutants::skip)]
    fn code_interpreter_cost(&self, _sessions: u32, _provider: &str) -> Decimal {
        Decimal::ZERO
    }
}

pub(crate) struct Decoder;

impl ContainerIdDecoder for Decoder {
    fn decode(
        &self,
        container_id: &str,
        provider: &str,
        params: &Object,
    ) -> Result<DecodedContainerCall, ContainerFieldTypeError> {
        let encoded = serde_json::to_value(params).map_err(|_error| params_field())?;
        let values: IndexMap<String, JsonValue> =
            serde_json::from_value(encoded).map_err(|_error| params_field())?;
        // ast-grep-ignore: no-long-names
        let decoded = decode_managed_container_id_for_request(container_id, provider, &values)?;
        let object =
            serde_json::to_value(&decoded.litellm_params).map_err(|_error| params_field())?;
        let litellm_params = serde_json::from_value(object).map_err(|_error| params_field())?;
        Ok(DecodedContainerCall {
            container_id: decoded.container_id,
            custom_llm_provider: decoded.custom_llm_provider,
            litellm_params,
        })
    }
}

pub(crate) const fn params_field() -> ContainerFieldTypeError {
    ContainerFieldTypeError {
        field: "litellm_params",
    }
}

pub(crate) struct NoPreCall;

impl PreCallHook for NoPreCall {
    fn pre_call(&self, _state: &mut LoggingState, _record: &PreCall) -> Result<(), PrePostError> {
        Ok(())
    }
}

// the session's uuid4 source and its clock: utcnow for the logged start, monotonic from the session's start
pub(crate) struct SessionIds;

impl IdSource for SessionIds {
    fn next_call_id(&self) -> String {
        Uuid::new_v4().to_string()
    }
}

pub(crate) struct SessionClock {
    started: Instant,
}

impl SessionClock {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl WrapperClock for SessionClock {
    fn now(&self) -> DateTime<FixedOffset> {
        Utc::now().fixed_offset()
    }

    fn monotonic(&self) -> Duration {
        self.started.elapsed()
    }
}

pub(crate) fn logging_state(
    call_type: &str,
    call_id: Option<&str>,
    redactor: &Redactor,
    ids: &dyn IdSource,
    clock: &dyn WrapperClock,
) -> Result<Mutex<LoggingState>, SessionError> {
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let state = LoggingState::new(
        init(call_type, call_id, ids, clock),
        setup,
        redactor,
        || ids.next_call_id(),
        Duration::default,
    )
    .map_err(|source| SessionError::Logging { source })?;
    Ok(Mutex::new(state))
}

pub(crate) fn init(
    call_type: &str,
    call_id: Option<&str>,
    ids: &dyn IdSource,
    clock: &dyn WrapperClock,
) -> LoggingInit {
    LoggingInit {
        model: String::new(),
        call_type: call_type.to_owned(),
        start_time: clock.now(),
        litellm_call_id: call_id.map_or_else(|| ids.next_call_id(), str::to_owned),
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
    }
}
