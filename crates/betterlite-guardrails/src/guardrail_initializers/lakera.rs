use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::LakeraCategoryThresholds;
use secrecy::SecretString;

mod tests;

#[derive(Debug, Clone)]
pub struct LakeraArgs {
    pub name: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub category_thresholds: Option<LakeraCategoryThresholds>,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
    pub event_hook: LitellmParamsMode,
}

#[must_use]
pub fn initialize_lakera(params: &LitellmParams, guardrail_name: Option<&str>) -> LakeraArgs {
    LakeraArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        api_base: params.base.fields.api_base.clone(),
        api_key: params.base.fields.api_key.clone(),
        category_thresholds: params.base.fields.category_thresholds,
        default_on: params.base.fields.default_on.unwrap_or_default(),
        timeout: params.base.fields.timeout,
        event_hook: params.mode.clone(),
    }
}
