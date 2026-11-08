use std::collections::BTreeMap;

use betterlite_context::bedrock_model::strip_bedrock_routing_prefix;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::value::RawValue;

use super::error::AwsEndpointError;
use super::provider::BedrockInvokeProvider;

// Python's urllib.parse.quote(model_id, safe="") leaves only letters, digits and _.-~ alone
pub const MODEL_ID_SAFE: AsciiSet = NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

pub fn bedrock_model_id(
    optional_params: &mut BTreeMap<String, Box<RawValue>>,
    provider: Option<BedrockInvokeProvider>,
    model: &str,
) -> Result<String, AwsEndpointError> {
    if let Some(raw) = optional_params.remove("model_id") {
        let override_id: Option<String> =
            serde_json::from_str(raw.get()).map_err(|source| AwsEndpointError::Field {
                field: "model_id",
                source,
            })?;
        if let Some(model_id) = override_id {
            return Ok(utf8_percent_encode(&model_id, &MODEL_ID_SAFE).to_string());
        }
    }
    let stripped = strip_bedrock_routing_prefix(model);
    if stripped.starts_with("arn:") {
        return Ok(utf8_percent_encode(&stripped, &MODEL_ID_SAFE).to_string());
    }
    let model_id = stripped.replacen("invoke/", "", 1);
    let specification = match provider {
        Some(BedrockInvokeProvider::Llama) if model_id.contains("llama/") => Some("llama"),
        Some(BedrockInvokeProvider::DeepseekR1) if model_id.contains("deepseek_r1/") => {
            Some("deepseek_r1")
        }
        Some(BedrockInvokeProvider::Openai) if model_id.contains("openai/") => Some("openai"),
        Some(BedrockInvokeProvider::Qwen2) if model_id.contains("qwen2/") => Some("qwen2"),
        Some(BedrockInvokeProvider::Qwen3) if model_id.contains("qwen3/") => Some("qwen3"),
        Some(BedrockInvokeProvider::Stability) if model_id.contains("stability/") => {
            Some("stability")
        }
        Some(BedrockInvokeProvider::Moonshot) if model_id.contains("moonshot/") => Some("moonshot"),
        _ if model_id.contains("nova-2/") => Some("nova-2"),
        _ if model_id.contains("nova/") => Some("nova"),
        _ => None,
    };
    match specification {
        Some(spec) => Ok(utf8_percent_encode(
            &model_id.replace(&format!("{spec}/"), ""),
            &MODEL_ID_SAFE,
        )
        .to_string()),
        None => Ok(model_id),
    }
}
