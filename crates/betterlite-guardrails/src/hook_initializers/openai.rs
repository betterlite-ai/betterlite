use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::error::HookInitError;
use super::{ParamMap, params_extras, read, required_name};

mod tests;

const VENDOR: &str = "OpenAI Moderation";
const END_OF_STREAM_ONLY: &str = "streaming_end_of_stream_only";
const SAMPLING_RATE: &str = "streaming_sampling_rate";

type ParamReader<T> = fn(&ParamMap, &'static str, &str) -> Result<Option<T>, HookInitError>;

#[derive(Debug, Clone)]
pub struct OpenAiArgs {
    pub guardrail_name: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub default_on: GuardrailFlag,
    pub event_hook: LitellmParamsMode,
    pub model: Option<String>,
    pub extras: OpaqueParams,
    pub streaming_end_of_stream_only: Option<bool>,
    pub streaming_sampling_rate: Option<i64>,
}

pub fn initialize_openai(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<OpenAiArgs, HookInitError> {
    let name = required_name(guardrail_name, VENDOR)?;
    let extra = &params.base.extra.0;
    let optional = read::<ParamMap>(extra, VENDOR, "optional_params")?;
    let api_key = params.base.fields.api_key.clone();
    let api_base = params.base.fields.api_base.clone();
    let default_on = params.base.fields.default_on.unwrap_or_default();
    let model = params.base.fields.model.clone();
    let extras = params_extras(
        params,
        VENDOR,
        api_key.as_ref(),
        api_base.as_deref(),
        default_on,
    )?;
    let streaming_end_of_stream_only = get_config_value(
        optional.as_ref(),
        extra,
        END_OF_STREAM_ONLY,
        read::<GuardrailFlag>,
    )?;
    let streaming_sampling_rate =
        get_config_value(optional.as_ref(), extra, SAMPLING_RATE, read::<i64>)?;
    Ok(OpenAiArgs {
        guardrail_name: name,
        api_key,
        api_base,
        default_on,
        event_hook: params.mode.clone(),
        model,
        extras,
        streaming_end_of_stream_only,
        streaming_sampling_rate,
    })
}

// LiteLLM's _get_config_value: optional_params wins when it holds a non-null value, then the param
fn get_config_value<T>(
    optional: Option<&ParamMap>,
    extra: &ParamMap,
    key: &str,
    read: ParamReader<T>,
) -> Result<Option<T>, HookInitError> {
    if let Some(map) = optional
        && let Some(value) = read(map, VENDOR, key)?
    {
        return Ok(Some(value));
    }
    read(extra, VENDOR, key)
}
