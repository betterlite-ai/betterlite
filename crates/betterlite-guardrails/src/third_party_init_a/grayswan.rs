use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::error::ThirdPartyInitAError;
use super::{InitResult, ParamMap, config_value, decode, optional_params};

mod tests;

const DEFAULT_SAMPLING_RATE: u64 = 5;
// LitellmParams reads optional_params through the Cisco model, whose on_flagged_action defaults to block
const CISCO_FLAGGED_ACTION: &str = "block";
// the top-level on_flagged_action is the Pillar mixin's field, which defaults to monitor
const PILLAR_FLAGGED_ACTION: &str = "monitor";

#[derive(Debug, Clone)]
pub struct GraySwanArgs {
    pub name: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub on_flagged_action: Option<String>,
    pub violation_threshold: Option<f64>,
    pub reasoning_mode: Option<String>,
    pub categories: Option<BTreeMap<String, String>>,
    pub policy_id: Option<String>,
    pub streaming_end_of_stream_only: bool,
    pub streaming_sampling_rate: u64,
    pub fail_open: Option<bool>,
    pub guardrail_timeout: Option<f64>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
}

fn flagged_action(optional: Option<&ParamMap>, extra: &ParamMap) -> InitResult<Option<String>> {
    if let Some(entries) = optional {
        match entries.get("on_flagged_action") {
            None => return Ok(Some(CISCO_FLAGGED_ACTION.to_owned())),
            Some(GuardrailValue::Null) => {}
            Some(entry) => return decode("on_flagged_action", Some(entry)),
        }
    }
    match extra.get("on_flagged_action") {
        None => Ok(Some(PILLAR_FLAGGED_ACTION.to_owned())),
        Some(top_level) => decode("on_flagged_action", Some(top_level)),
    }
}

pub fn initialize_grayswan(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<GraySwanArgs> {
    let name = guardrail_name
        .filter(|name| !name.is_empty())
        .ok_or(ThirdPartyInitAError::MissingGraySwanName)?;
    let fields = &params.base.fields;
    let extra = &params.base.extra.0;
    let optional = optional_params(&params.base.extra)?;
    let lookup = |key: &str| config_value(optional, extra, key);
    // an or-chain in LiteLLM: a false flag and a rate of 0 both fall to the default
    let end_of_stream_only: Option<bool> = decode(
        "streaming_end_of_stream_only",
        lookup("streaming_end_of_stream_only"),
    )?;
    let sampling_rate: Option<u64> =
        decode("streaming_sampling_rate", lookup("streaming_sampling_rate"))?;
    Ok(GraySwanArgs {
        name: name.to_owned(),
        api_key: fields.api_key.clone(),
        api_base: fields.api_base.clone(),
        on_flagged_action: flagged_action(optional, extra)?,
        violation_threshold: decode("violation_threshold", lookup("violation_threshold"))?,
        reasoning_mode: decode("reasoning_mode", lookup("reasoning_mode"))?,
        // the top-level categories is the content filter's list, so only optional_params can carry this dict
        categories: decode(
            "categories",
            optional.and_then(|entries| entries.get("categories")),
        )?,
        policy_id: decode("policy_id", lookup("policy_id"))?,
        streaming_end_of_stream_only: end_of_stream_only.unwrap_or_default(),
        streaming_sampling_rate: sampling_rate
            .filter(|rate| *rate != 0)
            .unwrap_or(DEFAULT_SAMPLING_RATE),
        fail_open: decode("fail_open", lookup("fail_open"))?,
        guardrail_timeout: decode("guardrail_timeout", lookup("guardrail_timeout"))?,
        event_hook: params.mode.clone(),
        default_on: fields.default_on.unwrap_or_default(),
    })
}
