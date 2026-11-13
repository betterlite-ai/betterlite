use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::error::InitializerError;

mod tests;

// unused until the secret detection class task lands; the enterprise package error is the only result
#[derive(Debug, Clone)]
pub struct HideSecretsArgs {
    pub name: String,
    pub event_hook: LitellmParamsMode,
    pub detect_secrets_config: Option<OpaqueParams>,
    pub default_on: GuardrailFlag,
}

// ast-grep-ignore: no-one-line-functions
pub const fn initialize_hide_secrets(
    _params: &LitellmParams,
    _guardrail_name: Option<&str>,
) -> Result<HideSecretsArgs, InitializerError> {
    Err(InitializerError::MissingEnterprisePackage)
}
