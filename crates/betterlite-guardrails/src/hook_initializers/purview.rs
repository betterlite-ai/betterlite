use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::HookInitError;
use super::{required_name, required_text, text};

mod tests;

const VENDOR: &str = "Microsoft Purview";
const DEFAULT_APP_NAME: &str = "LiteLLM";
const DEFAULT_USER_ID_FIELD: &str = "user_id";

#[derive(Debug, Clone)]
pub struct PurviewArgs {
    pub guardrail_name: String,
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: SecretString,
    pub purview_app_name: String,
    pub user_id_field: String,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
}

pub fn initialize_purview(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<PurviewArgs, HookInitError> {
    let extra = &params.base.extra.0;
    let tenant_id = required_text(extra, VENDOR, "tenant_id")?;
    let client_id = required_text(extra, VENDOR, "client_id")?;
    let configured_secret = text(extra, VENDOR, "client_secret")?;
    let client_secret = params
        .base
        .fields
        .api_key
        .clone()
        .filter(|key| !key.expose_secret().is_empty())
        .or_else(|| {
            configured_secret
                .filter(|value| !value.is_empty())
                .map(SecretString::from)
        })
        .ok_or(HookInitError::PurviewClientSecretRequired)?;
    let guardrail_name = required_name(guardrail_name, VENDOR)?;
    let purview_app_name = text(extra, VENDOR, "purview_app_name")?
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_APP_NAME.to_owned());
    let user_id_field = text(extra, VENDOR, "user_id_field")?
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_USER_ID_FIELD.to_owned());
    Ok(PurviewArgs {
        guardrail_name,
        tenant_id,
        client_id,
        client_secret,
        purview_app_name,
        user_id_field,
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
        timeout: params.base.fields.timeout,
    })
}
