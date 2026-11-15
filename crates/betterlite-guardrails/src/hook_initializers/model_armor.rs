use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

mod tests;

#[derive(Debug, Clone)]
pub struct ModelArmorArgs {
    pub guardrail_name: String,
    pub template_id: Option<String>,
    pub project_id: Option<GuardrailValue>,
    pub location: Option<String>,
    pub credentials: Option<SecretString>,
    pub api_endpoint: Option<String>,
    pub default_on: GuardrailFlag,
    pub mask_request_content: Option<bool>,
    pub mask_response_content: Option<bool>,
    pub fail_on_error: GuardrailFlag,
    pub skip_unscannable_attachments: GuardrailFlag,
    pub sanitize_error_detail: GuardrailFlag,
    pub timeout: Option<f64>,
    pub event_hook: LitellmParamsMode,
}

#[must_use]
pub fn initialize_model_armor(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> ModelArmorArgs {
    ModelArmorArgs {
        guardrail_name: guardrail_name.unwrap_or_default().to_owned(),
        template_id: params.base.fields.template_id.clone(),
        project_id: params
            .base
            .extra
            .0
            .get("project_id")
            .filter(|value| **value != GuardrailValue::Null)
            .cloned(),
        location: params.base.fields.location.clone(),
        credentials: params.base.fields.credentials.clone(),
        api_endpoint: params.base.fields.api_endpoint.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
        mask_request_content: params.base.fields.mask_request_content,
        mask_response_content: params.base.fields.mask_response_content,
        fail_on_error: params.base.fields.fail_on_error,
        skip_unscannable_attachments: params.base.handling_flags.skip_unscannable_attachments,
        sanitize_error_detail: params.base.fields.sanitize_error_detail,
        timeout: params.base.fields.timeout,
        event_hook: params.mode.clone(),
    }
}
