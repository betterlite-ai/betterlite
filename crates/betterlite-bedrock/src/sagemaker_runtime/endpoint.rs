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
    pub append_path_to_base: bool,
}

// _prepare_request and get_complete_url: the runtime endpoint, which sagemaker_base_url replaces whole
#[must_use]
pub fn runtime_url(request: &EndpointRequest<'_>) -> String {
    if let Some(base) = request.base_url {
        if !request.append_path_to_base {
            return base.to_owned();
        }
        let path = if request.stream {
            RESPONSE_STREAM
        } else {
            INVOCATIONS
        };
        let suffix = format!("/{path}");
        return if base.ends_with(&suffix) {
            base.to_owned()
        } else {
            format!("{}{}", base.trim_end_matches('/'), suffix)
        };
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

// _load_credentials: the call argument, then AWS_REGION_NAME, then AWS_REGION, then us-west-2.
// It has no model ARN step and no boto3 session, so the session region stays out of the rule.
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

// _get_aws_region_name: the model ARN is read before the environment, as the chat url and signer both do
pub fn chat_region(
    params: &ParamMap,
    model: &str,
    inputs: RegionInputs<'_>,
) -> Result<String, SigningError> {
    resolve_region(params, Some(model), inputs)
}
