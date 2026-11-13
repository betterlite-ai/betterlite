use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::panw_prisma_airs::FallbackAction;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::InitializerError;
use super::extras;

mod tests;

const DEFAULT_API_BASE: &str =
    "https://service.api.aisecurity.paloaltonetworks.com/v1/scan/sync/request";
const DEFAULT_TIMEOUT: f64 = 10.0;

#[derive(Debug, Clone)]
pub struct PanwArgs {
    pub name: String,
    pub api_key: SecretString,
    pub api_base: String,
    pub profile_name: String,
    pub default_on: GuardrailFlag,
    pub mask_on_block: GuardrailFlag,
    pub mask_request_content: GuardrailFlag,
    pub mask_response_content: GuardrailFlag,
    pub app_name: Option<String>,
    pub fallback_on_error: FallbackAction,
    pub timeout: f64,
    pub violation_message_template: Option<String>,
}

pub fn initialize_panw_prisma_airs(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<PanwArgs, InitializerError> {
    let extra = &params.base.extra;
    let api_key = params
        .base
        .fields
        .api_key
        .clone()
        .filter(|key| !key.expose_secret().is_empty())
        .ok_or(InitializerError::MissingApiKey)?;
    let profile_name = extras::read::<String>(extra, "profile_name", None)?
        .filter(|profile| !profile.is_empty())
        .ok_or(InitializerError::MissingProfileName)?;
    let api_base = match params.base.fields.api_base.clone() {
        Some(base) if !base.is_empty() => base,
        Some(_) | None => DEFAULT_API_BASE.to_owned(),
    };
    Ok(PanwArgs {
        name: guardrail_name.unwrap_or("panw_prisma_airs").to_owned(),
        api_key,
        api_base,
        profile_name,
        default_on: params.base.fields.default_on.unwrap_or_default(),
        mask_on_block: extras::flag(extra, "mask_on_block", false)?,
        mask_request_content: params.base.fields.mask_request_content.unwrap_or_default(),
        mask_response_content: params.base.fields.mask_response_content.unwrap_or_default(),
        app_name: extras::read(extra, "app_name", None)?,
        fallback_on_error: extras::read::<FallbackAction>(extra, "fallback_on_error", None)?
            .unwrap_or(FallbackAction::Block),
        timeout: params.base.fields.timeout.unwrap_or(DEFAULT_TIMEOUT),
        violation_message_template: params.base.fields.violation_message_template.clone(),
    })
}
