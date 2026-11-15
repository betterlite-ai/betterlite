use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::Client;
use secrecy::SecretString;
use serde::Deserialize;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_base::settings::GuardrailSettings;

pub const DEFAULT_LOCATION: &str = "us-central1";

pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 6] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::DuringMcpCall,
    GuardrailEventHooks::LoggingOnly,
];

const DEFAULT_EVENT_HOOKS: [&str; 3] = ["pre_call", "during_call", "post_call"];

// The admin UI schema LiteLLM returns from get_config_model.
#[derive(Debug, Clone, Deserialize)]
#[serde(from = "RawConfigModel")]
pub struct ModelArmorGuardrailConfigModel {
    pub template_id: Option<String>,
    pub project_id: Option<String>,
    pub location: Option<String>,
    pub credentials: Option<SecretString>,
    pub api_endpoint: Option<String>,
    pub fail_on_error: bool,
    pub sanitize_error_detail: bool,
    pub optional_params: Option<GuardrailValue>,
}

pub type ModelArmorConfigModel = ModelArmorGuardrailConfigModel;

#[derive(Deserialize)]
struct RawConfigModel {
    template_id: Option<String>,
    project_id: Option<String>,
    location: Option<String>,
    credentials: Option<SecretString>,
    api_endpoint: Option<String>,
    fail_on_error: Option<bool>,
    sanitize_error_detail: Option<bool>,
    optional_params: Option<GuardrailValue>,
}

impl From<RawConfigModel> for ModelArmorGuardrailConfigModel {
    fn from(raw: RawConfigModel) -> Self {
        Self {
            template_id: raw.template_id,
            project_id: raw.project_id,
            location: raw.location,
            credentials: raw.credentials,
            api_endpoint: raw.api_endpoint,
            fail_on_error: raw.fail_on_error.unwrap_or(true),
            sanitize_error_detail: raw.sanitize_error_detail.unwrap_or(true),
            optional_params: raw.optional_params,
        }
    }
}

impl GuardrailConfigModelIdentity for ModelArmorGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Google Cloud Model Armor"
    }
}

pub struct ModelArmorParams {
    pub template_id: Option<String>,
    pub project_id: Option<String>,
    pub location: Option<String>,
    pub credentials: Option<SecretString>,
    pub api_endpoint: Option<String>,
    pub sanitize_error_detail: Option<bool>,
    pub async_handler: Option<Client>,
    pub build_client: fn() -> Client,
    pub settings: GuardrailSettings,
    pub strict_modes: bool,
}

pub struct ModelArmorGuardrail {
    pub base: GuardrailBase,
    pub template_id: Option<String>,
    pub project_id: Option<String>,
    pub location: String,
    pub credentials: Option<SecretString>,
    pub api_endpoint: Option<String>,
    pub sanitize_error_detail: bool,
    pub async_handler: Client,
}

impl ModelArmorGuardrail {
    pub fn new(params: ModelArmorParams) -> Result<Self, GuardrailBaseError> {
        let mut settings = params.settings;
        if settings.event_hook.is_none() {
            settings.event_hook = Some(LitellmParamsMode::Many(
                DEFAULT_EVENT_HOOKS.map(String::from).to_vec(),
            ));
        }
        if settings.supported_event_hooks.is_none() {
            settings.supported_event_hooks = Some(SUPPORTED_EVENT_HOOKS.to_vec());
        }
        settings.switches.use_native_lifecycle_hooks = true;
        settings.switches.uses_apply_guardrail_interface = true;
        let base = GuardrailBase::new(settings, params.strict_modes)?;
        let location = params
            .location
            .unwrap_or_else(|| DEFAULT_LOCATION.to_owned());
        let async_handler = params.async_handler.unwrap_or_else(params.build_client);
        tracing::debug!(
            template_id = ?params.template_id,
            project_id = ?params.project_id,
            location = %location,
            "Model Armor Guardrail initialized"
        );
        Ok(Self {
            base,
            template_id: params.template_id,
            project_id: params.project_id,
            location,
            credentials: params.credentials,
            api_endpoint: params.api_endpoint,
            sanitize_error_detail: params.sanitize_error_detail.unwrap_or(true),
            async_handler,
        })
    }

    // LiteLLM re-normalizes the flag with `is not False`, so a null update keeps it sanitized.
    pub fn update_litellm_params(&mut self, params: &LitellmParams) {
        self.base.update_params(params);
        self.sanitize_error_detail = params.base.fields.sanitize_error_detail;
    }
}
