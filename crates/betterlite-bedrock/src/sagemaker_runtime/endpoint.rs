use std::collections::BTreeMap;

use betterlite_context::aws_partition::get_aws_partition;
use serde_json::value::RawValue;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::aws_endpoints::region::non_llm_region_name;
use crate::signing::error::SigningError;
use crate::signing::target::RegionInputs;

pub type ParamMap = BTreeMap<String, Box<RawValue>>;

const COMPLETION_PREFIX: &str = "sagemaker/";
const CHAT_PREFIX: &str = "sagemaker_chat/";
const NOVA_PREFIX: &str = "sagemaker_nova/";
const INVOCATIONS: &str = "invocations";
const RESPONSE_STREAM: &str = "invocations-response-stream";

#[must_use]
pub fn endpoint_name(model: &str) -> &str {
    model
        .strip_prefix(COMPLETION_PREFIX)
        .or_else(|| model.strip_prefix(CHAT_PREFIX))
        .or_else(|| model.strip_prefix(NOVA_PREFIX))
        .unwrap_or(model)
}

#[derive(Debug, Clone, Copy)]
pub struct EndpointRequest<'request> {
    pub model: &'request str,
    pub region: &'request str,
    pub base_url: Option<&'request str>,
    pub stream: bool,
}

#[must_use]
pub fn runtime_url(request: &EndpointRequest<'_>) -> String {
    if let Some(base) = request.base_url {
        return base.to_owned();
    }
    let suffix = get_aws_partition(Some(request.region)).dns_suffix;
    let path = if request.stream {
        RESPONSE_STREAM
    } else {
        INVOCATIONS
    };
    format!(
        "https://runtime.sagemaker.{}.{}/endpoints/{}/{}",
        request.region,
        suffix,
        endpoint_name(request.model),
        path
    )
}

pub fn call_region(params: &ParamMap, inputs: RegionInputs<'_>) -> Result<String, SigningError> {
    let explicit = params.get("aws_region_name").map_or(Ok(None), |raw| {
        serde_json::from_str::<Option<String>>(raw.get()).map_err(|source| {
            SigningError::Endpoint(AwsEndpointError::Field {
                field: "aws_region_name",
                source,
            })
        })
    })?;
    non_llm_region_name(
        explicit.as_deref(),
        inputs.environment_region_name,
        inputs.environment_region,
    )
    .map_err(SigningError::Endpoint)
}
