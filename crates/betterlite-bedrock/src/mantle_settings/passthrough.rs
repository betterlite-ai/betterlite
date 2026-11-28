use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{PassthroughTarget, RelayedResponse};
use secrecy::SecretString;

use crate::aws_endpoints::runtime::{
    BedrockEndpointType, RuntimeEndpoint, RuntimeEndpointInput, runtime_endpoint,
};
use crate::aws_settings::or_bearer_token;
use crate::mantle_inputs::records::MantleGlobals;
use crate::mantle_inputs::region::{region_from_host, resolve_region};
use crate::passthrough::error::PassthroughFailure;

use super::error::MantleSettingsError;
use super::field_text;

pub const CONVERSE_PROVIDER: ProviderId = ProviderId::Bedrock;

pub struct PassthroughEndpointRequest<'text> {
    pub api_base: Option<&'text str>,
    pub configured_endpoint: Option<&'text str>,
    pub environment_endpoint: Option<&'text str>,
    pub region: &'text str,
    pub endpoint_type: BedrockEndpointType,
}

// BedrockMantlePassthroughConfig._get_aws_region_name: the shared region chain, with no model or model_id input to ignore.
pub fn passthrough_region(
    optional_params: &Object,
    globals: &MantleGlobals,
) -> Result<String, MantleSettingsError> {
    let aws_region_name = field_text(optional_params, "aws_region_name")?;
    let api_base = field_text(optional_params, "api_base")?;
    resolve_region(aws_region_name.as_deref(), api_base.as_deref(), globals)
        .map_err(MantleSettingsError::from)
}

// A recognized Mantle base serves only the OpenAI surface, so the request itself goes to the runtime endpoint while keeping the resolved region.
#[must_use]
pub fn passthrough_runtime_endpoint(request: &PassthroughEndpointRequest<'_>) -> RuntimeEndpoint {
    let api_base = request
        .api_base
        .filter(|base| region_from_host(base).is_none());
    runtime_endpoint(RuntimeEndpointInput {
        api_base,
        configured_endpoint: request.configured_endpoint,
        environment_endpoint: request.environment_endpoint,
        region: request.region,
        endpoint_type: request.endpoint_type,
    })
}

// BedrockMantlePassthroughConfig.get_bedrock_bearer_token accepts only a string api_key; anything else is absent and the env chain still applies.
#[must_use]
pub fn passthrough_bearer_token(
    litellm_params: &Object,
    globals: &MantleGlobals,
) -> Option<SecretString> {
    let api_key = litellm_params
        .get("api_key")
        .and_then(|value| value.as_str())
        .map(|text| SecretString::from(text.to_owned()));
    or_bearer_token(&[
        api_key.as_ref(),
        globals.mantle_api_key.as_ref(),
        globals.aws_bearer_token.as_ref(),
    ])
}

// Only a converse endpoint without invoke is logged with the native Bedrock reply shape; invoke wins when both substrings occur.
#[must_use]
pub fn reply_shape_provider(custom_llm_provider: &str, endpoint: &str) -> String {
    if !endpoint.contains("invoke") && endpoint.contains("converse") {
        return CONVERSE_PROVIDER.to_string();
    }
    custom_llm_provider.to_owned()
}

// BedrockMantlePassthroughConfig.logging_non_streaming_response: the shaped provider travels on with every other argument to B658 unchanged.
pub fn logging_non_streaming_response<Forward, Logged>(
    target: PassthroughTarget<'_>,
    response: &RelayedResponse<'_>,
    request_data: &Object,
    forward: Forward,
) -> Result<Option<Logged>, PassthroughFailure>
where
    Forward: FnOnce(
        PassthroughTarget<'_>,
        &RelayedResponse<'_>,
        &Object,
    ) -> Result<Option<Logged>, PassthroughFailure>,
{
    let provider = reply_shape_provider(target.custom_llm_provider, target.endpoint);
    let shaped = PassthroughTarget {
        custom_llm_provider: &provider,
        ..target
    };
    forward(shaped, response, request_data)
}
