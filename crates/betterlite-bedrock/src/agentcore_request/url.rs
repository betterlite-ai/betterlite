use betterlite_context::aws_partition::get_aws_partition;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use super::arn::{agent_runtime_arn, region_from_arn};
use super::error::AgentCoreRequestError;
use crate::model_routes::{ParamMap, ParamValue};

// Python's quote(safe="") leaves letters, digits and _.-~ alone and percent-encodes every other UTF-8 byte
const ARN_BYTES: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

pub fn complete_url(
    model: &str,
    optional_params: &ParamMap,
) -> Result<String, AgentCoreRequestError> {
    let arn = agent_runtime_arn(model)?;
    let region = region_from_arn(arn)?;
    let base = runtime_endpoint(optional_params)?.map_or_else(
        || {
            format!(
                "https://bedrock-agentcore.{region}.{}",
                get_aws_partition(Some(region)).dns_suffix
            )
        },
        str::to_owned,
    );
    let mut url = format!(
        "{base}/runtimes/{}/invocations",
        utf8_percent_encode(arn, ARN_BYTES)
    );
    if let Some(value) = optional_params.get("qualifier") {
        url.push_str("?qualifier=");
        url.push_str(&qualifier_text(value)?);
    }
    Ok(url)
}

fn runtime_endpoint(optional_params: &ParamMap) -> Result<Option<&str>, AgentCoreRequestError> {
    match optional_params.get("aws_bedrock_runtime_endpoint") {
        None | Some(ParamValue::Null) => Ok(None),
        Some(ParamValue::Text(text)) if text.is_empty() => Ok(None),
        Some(ParamValue::Text(text)) => Ok(Some(text)),
        Some(_) => Err(AgentCoreRequestError::WrongType {
            field: "aws_bedrock_runtime_endpoint",
        }),
    }
}

// LiteLLM interpolates str(qualifier) without URL-encoding it, so None reaches the query as "None"
fn qualifier_text(value: &ParamValue) -> Result<String, AgentCoreRequestError> {
    match value {
        ParamValue::Null => Ok("None".to_owned()),
        ParamValue::Text(text) => Ok(text.clone()),
        ParamValue::Bool(_)
        | ParamValue::Number(_)
        | ParamValue::List(_)
        | ParamValue::Mapping(_) => Err(AgentCoreRequestError::WrongType { field: "qualifier" }),
    }
}
