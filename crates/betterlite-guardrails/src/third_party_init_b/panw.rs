use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::base::BaseLitellmParams;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::InitResult;
use super::error::ThirdPartyInitBError;

mod tests;

const SECRET_PARAMS: [&str; 2] = ["api_key", "credentials"];

#[derive(Clone, Debug)]
pub enum SetParam {
    Value(GuardrailValue),
    Secret(SecretString),
}

#[derive(Debug, Clone)]
pub struct PanwFolderArgs {
    pub name: String,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub set_params: BTreeMap<String, SetParam>,
}

pub fn initialize_panw_prisma_airs(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<PanwFolderArgs> {
    let name = guardrail_name
        .filter(|name| !name.is_empty())
        .ok_or(ThirdPartyInitBError::MissingPanwName)?;
    let set_params = sent_params(&params.base)?;
    Ok(PanwFolderArgs {
        name: name.to_owned(),
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
        set_params,
    })
}

// LiteLLM's model_dump(exclude_unset=True) passes every param the config wrote and no default
fn sent_params(base: &BaseLitellmParams) -> InitResult<BTreeMap<String, SetParam>> {
    let sent = base
        .sent_fields()
        .map_err(|source| ThirdPartyInitBError::Params { source })?;
    sent.into_iter()
        .map(|(name, value)| match value {
            GuardrailValue::Text(text) if SECRET_PARAMS.contains(&name.as_str()) => {
                Ok((name, SetParam::Secret(SecretString::from(text))))
            }
            other => Ok((name, SetParam::Value(other))),
        })
        .collect()
}
