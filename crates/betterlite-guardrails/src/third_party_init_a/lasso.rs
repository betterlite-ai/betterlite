use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::{InitResult, decode};

mod tests;

#[derive(Debug, Clone)]
pub struct LassoFolderArgs {
    pub name: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub user_id: Option<String>,
    pub conversation_id: Option<String>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
}

pub fn initialize_lasso_folder(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<LassoFolderArgs> {
    let fields = &params.base.fields;
    let extra = &params.base.extra.0;
    Ok(LassoFolderArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        api_key: fields.api_key.clone(),
        api_base: fields.api_base.clone(),
        user_id: decode("lasso_user_id", extra.get("lasso_user_id"))?,
        conversation_id: decode("lasso_conversation_id", extra.get("lasso_conversation_id"))?,
        event_hook: params.mode.clone(),
        default_on: fields.default_on.unwrap_or_default(),
        timeout: fields.timeout,
    })
}
