use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::error::InitializerError;
use super::extras;

mod tests;

#[derive(Debug, Clone)]
pub struct LassoArgs {
    pub name: String,
    pub lasso_api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub user_id: Option<String>,
    pub conversation_id: Option<String>,
    pub mask: Option<bool>,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
    pub event_hook: LitellmParamsMode,
}

pub fn initialize_lasso(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<LassoArgs, InitializerError> {
    let extra = &params.base.extra;
    Ok(LassoArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        lasso_api_key: params.base.fields.api_key.clone(),
        api_base: params.base.fields.api_base.clone(),
        user_id: extras::read(extra, "lasso_user_id", None)?,
        conversation_id: extras::read(extra, "lasso_conversation_id", None)?,
        mask: extras::read(extra, "mask", Some(false))?,
        default_on: params.base.fields.default_on.unwrap_or_default(),
        timeout: params.base.fields.timeout,
        event_hook: params.mode.clone(),
    })
}
