use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::{InitResult, config_value, decode, optional_params};
use crate::generic_guardrail_contracts::StreamingTransformMode;

mod tests;

#[derive(Debug, Clone)]
pub struct GenericApiArgs {
    pub name: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub headers: Option<BTreeMap<String, SecretString>>,
    pub provider_specific_params: OpaqueParams,
    pub unreachable_fallback: UnreachableFallback,
    pub fail_on_error: GuardrailFlag,
    pub extra_headers: Option<Vec<String>>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub streaming_end_of_stream_only: Option<bool>,
    pub streaming_sampling_rate: Option<u32>,
    pub streaming_transform_mode: Option<StreamingTransformMode>,
    pub timeout: Option<f64>,
}

pub fn initialize_generic_api(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<GenericApiArgs> {
    let fields = &params.base.fields;
    let extra = &params.base.extra.0;
    let optional = optional_params(&params.base.extra)?;
    let streaming = |name: &str| config_value(optional, extra, name);
    Ok(GenericApiArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        api_base: fields.api_base.clone(),
        api_key: fields.api_key.clone(),
        headers: decode("headers", extra.get("headers"))?,
        provider_specific_params: fields.provider_specific_params.clone().unwrap_or_default(),
        unreachable_fallback: fields.unreachable_fallback,
        fail_on_error: fields.fail_on_error,
        extra_headers: fields.extra_headers.clone(),
        event_hook: params.mode.clone(),
        default_on: fields.default_on.unwrap_or_default(),
        streaming_end_of_stream_only: decode(
            "streaming_end_of_stream_only",
            streaming("streaming_end_of_stream_only"),
        )?,
        streaming_sampling_rate: decode(
            "streaming_sampling_rate",
            streaming("streaming_sampling_rate"),
        )?,
        streaming_transform_mode: decode(
            "streaming_transform_mode",
            streaming("streaming_transform_mode"),
        )?,
        timeout: fields.timeout,
    })
}
