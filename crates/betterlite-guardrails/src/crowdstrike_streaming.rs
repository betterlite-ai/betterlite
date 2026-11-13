use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::params::LitellmParams;

use crate::crowdstrike_aidr::error::CrowdStrikeError;

pub const DEFAULT_SAMPLING_RATE: u64 = 5;
const RATE_KEY: &str = "streaming_sampling_rate";

const BUFFER_KEY: &str = "streaming_buffer_until_moderated";
const RELEASE_KEY: &str = "streaming_buffer_release_on_scan";
const END_OF_STREAM_KEY: &str = "streaming_end_of_stream_only";

// The four optional fields of LiteLLM's CrowdStrikeAIDRGuardrailConfigModelOptionalParams
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamingParams {
    pub buffer_until_moderated: Option<bool>,
    pub buffer_release_on_scan: Option<bool>,
    pub end_of_stream_only: Option<bool>,
    pub sampling_rate: Option<u64>,
}

// LiteLLM's _set_streaming_params: an unset flag is false and an unset rate is 5
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingSettings {
    pub buffer_until_moderated: bool,
    pub buffer_release_on_scan: bool,
    pub end_of_stream_only: bool,
    pub sampling_rate: u64,
}

impl Default for StreamingSettings {
    fn default() -> Self {
        Self {
            buffer_until_moderated: false,
            buffer_release_on_scan: false,
            end_of_stream_only: false,
            sampling_rate: DEFAULT_SAMPLING_RATE,
        }
    }
}

impl StreamingSettings {
    #[must_use]
    pub fn from_params(params: StreamingParams) -> Self {
        Self {
            buffer_until_moderated: params.buffer_until_moderated.unwrap_or_default(),
            buffer_release_on_scan: params.buffer_release_on_scan.unwrap_or_default(),
            end_of_stream_only: params.end_of_stream_only.unwrap_or_default(),
            sampling_rate: params.sampling_rate.unwrap_or(DEFAULT_SAMPLING_RATE),
        }
    }
}

// LiteLLM's streaming_params_from_litellm_params, preferring the explicit optional_params model
pub fn read_streaming_params(params: &LitellmParams) -> Result<StreamingParams, CrowdStrikeError> {
    let extras = &params.base.extra.0;
    let nested = optional_params(params)?;
    Ok(StreamingParams {
        buffer_until_moderated: bool_param(BUFFER_KEY, nested, extras)?,
        buffer_release_on_scan: bool_param(RELEASE_KEY, nested, extras)?,
        end_of_stream_only: bool_param(END_OF_STREAM_KEY, nested, extras)?,
        sampling_rate: rate_param(nested, extras)?,
    })
}

fn optional_params(
    params: &LitellmParams,
) -> Result<Option<&BTreeMap<String, GuardrailValue>>, CrowdStrikeError> {
    match params.base.extra.0.get("optional_params") {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Object(fields)) => Ok(Some(fields)),
        Some(_wrong) => Err(CrowdStrikeError::InvalidParam {
            name: "optional_params",
        }),
    }
}

fn selected<'value>(
    name: &str,
    nested: Option<&'value BTreeMap<String, GuardrailValue>>,
    extras: &'value BTreeMap<String, GuardrailValue>,
) -> Option<&'value GuardrailValue> {
    match nested.and_then(|fields| fields.get(name)) {
        Some(value) => Some(value),
        None => extras.get(name),
    }
}

fn bool_param(
    name: &'static str,
    nested: Option<&BTreeMap<String, GuardrailValue>>,
    extras: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<bool>, CrowdStrikeError> {
    match selected(name, nested, extras) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Boolean(flag)) => Ok(Some(*flag)),
        Some(_wrong) => Err(CrowdStrikeError::InvalidParam { name }),
    }
}

fn rate_param(
    nested: Option<&BTreeMap<String, GuardrailValue>>,
    extras: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<u64>, CrowdStrikeError> {
    let rate = match selected(RATE_KEY, nested, extras) {
        None | Some(GuardrailValue::Null) => return Ok(None),
        Some(GuardrailValue::SignedInteger(rate)) => u64::try_from(*rate)
            .map_err(|_negative| CrowdStrikeError::InvalidParam { name: RATE_KEY })?,
        Some(GuardrailValue::UnsignedInteger(rate)) => *rate,
        Some(_wrong) => {
            return Err(CrowdStrikeError::InvalidParam { name: RATE_KEY });
        }
    };
    if rate == 0 {
        return Err(CrowdStrikeError::InvalidParam { name: RATE_KEY });
    }
    Ok(Some(rate))
}

mod tests;
