use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::InitResult;
use super::error::ThirdPartyInitAError;
use crate::crowdstrike_streaming::read_streaming_params;

mod tests;

#[derive(Debug, Clone)]
pub struct CrowdStrikeArgs {
    pub name: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub fail_on_error: GuardrailFlag,
    pub buffer_until_moderated: Option<bool>,
    pub buffer_release_on_scan: Option<bool>,
    pub end_of_stream_only: Option<bool>,
    pub sampling_rate: Option<u64>,
    pub timeout: Option<f64>,
}

pub fn initialize_crowdstrike(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<CrowdStrikeArgs> {
    let name = guardrail_name
        .filter(|name| !name.is_empty())
        .ok_or(ThirdPartyInitAError::MissingCrowdStrikeName)?;
    let streaming =
        read_streaming_params(params).map_err(|source| ThirdPartyInitAError::Streaming {
            source: Box::new(source),
        })?;
    let fields = &params.base.fields;
    Ok(CrowdStrikeArgs {
        name: name.to_owned(),
        api_base: fields.api_base.clone(),
        api_key: fields.api_key.clone(),
        event_hook: params.mode.clone(),
        default_on: fields.default_on.unwrap_or_default(),
        fail_on_error: fields.fail_on_error,
        buffer_until_moderated: streaming.buffer_until_moderated,
        buffer_release_on_scan: streaming.buffer_release_on_scan,
        end_of_stream_only: streaming.end_of_stream_only,
        sampling_rate: streaming.sampling_rate,
        timeout: fields.timeout,
    })
}
