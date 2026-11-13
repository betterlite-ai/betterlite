use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;

mod tests;

#[derive(Debug, Clone)]
pub struct QohashArgs {
    pub api_base: Option<String>,
    pub name: String,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    // LiteLLM's additional_provider_specific_params, the name its litellm_params key keeps
    pub provider_params: Option<OpaqueParams>,
    pub extra_headers: Option<Vec<String>>,
    pub timeout: Option<f64>,
}

// LiteLLM's qohash/__init__.py initialize_guardrail passes only these fields to QostodianNexus
#[must_use]
pub fn initialize_qohash(params: &LitellmParams, guardrail_name: Option<&str>) -> QohashArgs {
    QohashArgs {
        api_base: params.base.fields.api_base.clone(),
        name: guardrail_name.unwrap_or_default().to_owned(),
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
        provider_params: params.base.fields.provider_specific_params.clone(),
        extra_headers: params.base.fields.extra_headers.clone(),
        timeout: params.base.fields.timeout,
    }
}
