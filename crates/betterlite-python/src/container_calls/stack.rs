use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Debug, Formatter};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::openai_credentials::Settings;
use betterlite_config::types::ProxyTrust;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::api::container_file_endpoints::handler::{
    ContainerConfigLookup, ContainerIdDecoder, DecodedContainerCall,
};
use betterlite_core::containers::container_request_parameters::{
    ContainerFieldTypeError, decode_managed_container_id_for_request,
};
use betterlite_core::cost::completion::lookup::{CatalogLookup, LookupSettings};
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_metadata_resolution::names::NameNormalizers;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::tool_fees::scalar::code_interpreter_fee;
use betterlite_openai::api::container_contracts::config::BaseContainerConfig;
use betterlite_openai::containers::OpenAiContainerConfig;
use betterlite_openai::containers::container_file_http::PreCallHook;
use betterlite_openai::containers::costs::ContainerCosts;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::Utc;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::SecretString;
use uuid::Uuid;

use super::error::SessionError;

// the outbound client honours the proxy variables of the environment the session was built from
pub(crate) fn outbound_client(
    environment: &BTreeMap<String, String>,
) -> Result<ProxiedClient, SessionError> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), environment)
        .map_err(|source| SessionError::Proxy { source })?;
    ProxiedClient::outbound(&Arc::new(policy)).map_err(|source| SessionError::Client { source })
}

pub(crate) fn redactor() -> Result<Redactor, SessionError> {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|source| SessionError::Redact { source })
}

// LiteLLM's manager answers a container config for openai and for no other provider here
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
        (provider == ProviderId::Openai).then_some(&self.openai as &dyn BaseContainerConfig)
    }
}

// get_cost_for_code_interpreter over the bundled price map; a lookup that fails reads as free, as LiteLLM's None does
pub(crate) struct CatalogCosts {
    lookup: CatalogLookup,
}

impl CatalogCosts {
    pub(crate) fn new(environment: &BTreeMap<String, String>) -> Result<Self, SessionError> {
        let bytes = (BundledCatalog::default().asset)()
            .map_err(|source| SessionError::Catalog { source })?;
        let settings = LookupSettings {
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: environment.clone(),
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
            normalizers: NameNormalizers {
                bedrock_base_model: get_bedrock_base_model,
                bedrock_routing_prefix: strip_bedrock_routing_prefix,
                mantle_region_split: split_region_prefix,
            },
        };
        let lookup = CatalogLookup::new(&bytes, BundledCatalog::default().get(), settings)
            .map_err(|source| SessionError::Lookup { source })?;
        Ok(Self { lookup })
    }
}

impl ContainerCosts for CatalogCosts {
    fn code_interpreter_cost(&self, sessions: u32, provider: &str) -> Decimal {
        let count = i64::from(sessions);
        code_interpreter_fee(Some(count), Some(provider), None, &self.lookup)
            .unwrap_or(Decimal::ZERO)
    }
}

impl Debug for CatalogCosts {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CatalogCosts")
            .finish_non_exhaustive()
    }
}

// B3361's managed id, read through the JSON map the request parameters store
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

const fn params_field() -> ContainerFieldTypeError {
    ContainerFieldTypeError {
        field: "litellm_params",
    }
}

// LiteLLM's container calls log through their logging object and run no custom pre-call hook
pub(crate) struct NoPreCall;

impl PreCallHook for NoPreCall {
    fn pre_call(&self, _state: &mut LoggingState, _record: &PreCall) -> Result<(), PrePostError> {
        Ok(())
    }
}

// one logging object per call, as the @client wrapper makes one for every container call it wraps
pub(crate) fn logging_state(
    call_type: &str,
    call_id: Option<&str>,
    redactor: &Redactor,
) -> Result<Mutex<LoggingState>, SessionError> {
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let state = LoggingState::new(
        init(call_type, call_id),
        setup,
        redactor,
        || Uuid::new_v4().to_string(),
        Duration::default,
    )
    .map_err(|source| SessionError::Logging { source })?;
    Ok(Mutex::new(state))
}

fn init(call_type: &str, call_id: Option<&str>) -> LoggingInit {
    LoggingInit {
        model: String::new(),
        call_type: call_type.to_owned(),
        start_time: Utc::now().fixed_offset(),
        litellm_call_id: call_id.map_or_else(|| Uuid::new_v4().to_string(), str::to_owned),
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
