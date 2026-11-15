use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::{LitellmParamsMode, Mode};
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::IntoDeserializer;
use serde::de::value::{Error as ValueError, StrDeserializer};

use super::error::ThirdPartyInitAError;
use super::{InitResult, decode};

mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadroomEventHook {
    Tagged(Mode),
    Many(Vec<GuardrailEventHooks>),
    One(GuardrailEventHooks),
}

#[derive(Debug, Clone)]
pub struct HeadroomArgs {
    pub name: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub model: Option<String>,
    pub event_hook: HeadroomEventHook,
    pub default_on: GuardrailFlag,
    pub unreachable_fallback: UnreachableFallback,
    pub timeout: Option<f64>,
    pub ccr_retrieval: bool,
}

fn hook(name: &str) -> InitResult<GuardrailEventHooks> {
    let deserializer: StrDeserializer<'_, ValueError> = name.into_deserializer();
    GuardrailEventHooks::deserialize(deserializer).map_err(|_unknown| {
        ThirdPartyInitAError::InvalidEventHook {
            hook: name.to_owned(),
        }
    })
}

// LiteLLM's _coerce_event_hook: a Mode stays, and every name becomes a GuardrailEventHooks member
fn coerce_event_hook(mode: &LitellmParamsMode) -> InitResult<HeadroomEventHook> {
    match mode {
        LitellmParamsMode::Tagged(tagged) => Ok(HeadroomEventHook::Tagged(tagged.clone())),
        LitellmParamsMode::Many(names) => names
            .iter()
            .map(String::as_str)
            .map(hook)
            .collect::<InitResult<Vec<_>>>()
            .map(HeadroomEventHook::Many),
        LitellmParamsMode::One(name) => hook(name).map(HeadroomEventHook::One),
    }
}

pub fn initialize_headroom(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<HeadroomArgs> {
    let name = guardrail_name.ok_or(ThirdPartyInitAError::MissingHeadroomName)?;
    let fields = &params.base.fields;
    let ccr_retrieval = decode("ccr_retrieval", params.base.extra.0.get("ccr_retrieval"))?;
    Ok(HeadroomArgs {
        name: name.to_owned(),
        api_base: fields.api_base.clone(),
        api_key: fields.api_key.clone(),
        model: fields.model.clone(),
        event_hook: coerce_event_hook(&params.mode)?,
        default_on: fields.default_on.unwrap_or_default(),
        unreachable_fallback: fields.unreachable_fallback,
        timeout: fields.timeout,
        ccr_retrieval: ccr_retrieval.unwrap_or(true),
    })
}
