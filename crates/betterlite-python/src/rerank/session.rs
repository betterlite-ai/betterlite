use super::convert::{ConvertError, RerankSettingsWire, SessionWire};
use super::error::SessionError;
use crate::completion::setup::{ModuleCredentials, outbound_client};
use betterlite_config::api::rerank_provider_plan::{Environment, RerankSettings};
use betterlite_config::loader::startup::process_environment;
use betterlite_config::sections::{cohere_rerank_settings, jina_infinity_rerank};
use betterlite_http::proxy::ProxiedClient;
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
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

// Everything one Python session owns: the environment snapshot, the HTTP client and the catalog it resolves against.
pub(crate) struct RerankSession {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) catalog: Arc<ModelCatalog>,
    pub(crate) known_model_names: BTreeSet<String>,
    pub(crate) custom_providers: BTreeSet<String>,
    pub(crate) registry: JSONProviderRegistry,
    pub(crate) client: ProxiedClient,
    pub(crate) redactor: Redactor,
}

impl RerankSession {
    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire = serde_json::from_str(text)
            .map_err(|source| SessionError::from(ConvertError::Json { source }))?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let client = outbound_client(&environment)?;
        let catalog = BundledCatalog::default().get();
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
        Ok(Self {
            environment,
            catalog,
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            registry: JSONProviderRegistry::load(),
            client,
            redactor,
        })
    }

    // litellm_settings as this call sees them: the module-level values plus the provider variables the branches read
    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn settings(&self, wire: &RerankSettingsWire) -> RerankSettings {
        RerankSettings {
            api_key: secret_text(wire.api_key.as_ref()),
            api_base: wire.api_base.clone().filter(|text| !text.is_empty()),
            headers: wire.headers.clone().filter(|headers| !headers.is_empty()),
            togetherai_api_key: secret_text(wire.togetherai_api_key.as_ref()),
            drop_params: wire.drop_params.unwrap_or(false),
            environment: Environment::default().resolve(&self.environment),
        }
    }

    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn cohere_settings(
        &self,
        wire: &RerankSettingsWire,
    ) -> cohere_rerank_settings::Settings {
        cohere_rerank_settings::Settings {
            cohere_api_key: environment_secret(&self.environment, "COHERE_API_KEY"),
            co_api_key: environment_secret(&self.environment, "CO_API_KEY"),
            cohere_key: secret_text(wire.cohere_key.as_ref()),
        }
    }

    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn host_settings(&self) -> jina_infinity_rerank::Settings {
        jina_infinity_rerank::Settings {
            infinity_api_key: environment_secret(&self.environment, "INFINITY_API_KEY"),
            infinity_key: None,
        }
    }

    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn context(&self, globals: &GlobalCredentials) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: ProviderPolicies::default(),
            globals: globals.clone(),
        }
    }

    // LiteLLM's logging object, built before the provider runs so update_from_kwargs sees the call's kwargs
    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn logging(
        &self,
        call_type: &str,
        model: &str,
        call_id: &str,
        kwargs: &IndexMap<String, Json>,
    ) -> Result<LoggingState, SessionError> {
        let init = LoggingInit {
            model: model.to_owned(),
            call_type: call_type.to_owned(),
            start_time: Utc::now().fixed_offset(),
            litellm_call_id: call_id.to_owned(),
            function_id: String::new(),
            messages: None,
            stream: None,
            litellm_trace_id: None,
            kwargs: kwargs.clone(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        };
        let mut context = CorrelationContext::default();
        LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut context,
                settings: CorrelationSettings::default(),
            },
            &self.redactor,
            || Uuid::new_v4().to_string(),
            Duration::default,
        )
        .map_err(SessionError::from)
    }
}

impl ModuleCredentials for RerankSettingsWire {
    #[cfg_attr(test, mutants::skip)]
    fn api_key(&self) -> Option<SecretString> {
        secret_text(self.api_key.as_ref())
    }

    #[cfg_attr(test, mutants::skip)]
    fn api_base(&self) -> Option<String> {
        self.api_base.clone().filter(|text| !text.is_empty())
    }
}

#[cfg_attr(test, mutants::skip)]
fn environment_secret(environment: &BTreeMap<String, String>, name: &str) -> Option<SecretString> {
    environment
        .get(name)
        .filter(|value| !value.is_empty())
        .map(|value| SecretString::from(value.clone()))
}

// LiteLLM's `value or next`: an empty key counts as unset
#[cfg_attr(test, mutants::skip)]
pub(crate) fn secret_text(value: Option<&SecretString>) -> Option<SecretString> {
    value
        .filter(|secret| !secret.expose_secret().is_empty())
        .cloned()
}
