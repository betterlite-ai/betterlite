use std::collections::BTreeMap;

use betterlite_context::aws_partition::get_aws_partition;
use serde_json::value::RawValue;

use crate::signing::error::SigningError;
use crate::signing::target::{RegionInputs, resolve_region};

pub type ParamMap = BTreeMap<String, Box<RawValue>>;

const COMPLETION_PREFIX: &str = "sagemaker/";
const CHAT_PREFIX: &str = "sagemaker_chat/";
const NOVA_PREFIX: &str = "sagemaker_nova/";
const INVOCATIONS: &str = "invocations";
const RESPONSE_STREAM: &str = "invocations-response-stream";

// the core dispatch removes the provider prefix before the handler sees the model; the runtime removes it too
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

// _prepare_request and get_complete_url: the runtime endpoint, which sagemaker_base_url replaces whole
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

// Completion skips model-ARN and boto3-session region sources.
pub fn call_region(params: &ParamMap, inputs: RegionInputs<'_>) -> Result<String, SigningError> {
    resolve_region(
        params,
        None,
        RegionInputs {
            session_region: None,
            ..inputs
        },
    )
}
