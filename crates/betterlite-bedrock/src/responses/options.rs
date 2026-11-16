use std::collections::BTreeSet;

use betterlite_core::api::codex_history_tools::drop_unsupported_tools;
use betterlite_logging::analytics::python::repr;
use betterlite_openai::reasoning_models::DropParams;
use betterlite_openai::response_params;
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::capabilities::ResponsesCapabilities;
use betterlite_openai::response_params::params::{ResponsesOptions, ResponsesParams};
use betterlite_prompts::arguments::json::{Json, Object};

use crate::responses::config::CUSTOM_LLM_PROVIDER;
use crate::responses::error::ResponsesError;

// The set bedrock-runtime's own validation error names
const SUPPORTED_TOOL_TYPES: [&str; 7] = [
    "apply_patch",
    "computer",
    "custom",
    "function",
    "mcp",
    "namespace",
    "tool_search",
];
const SUPPORTED_TOOL_TYPES_REPR: &str =
    "['apply_patch', 'computer', 'custom', 'function', 'mcp', 'namespace', 'tool_search']";

pub async fn map_openai_params<C>(
    capabilities: &mut C,
    model: &str,
    params: &ResponsesParams,
    drop_params: DropParams,
    bundled: &mut BundledReasoningModels,
) -> Result<Object, ResponsesError>
where
    C: ResponsesCapabilities,
{
    let options = ResponsesOptions {
        provider: CUSTOM_LLM_PROVIDER,
        model: model.to_owned(),
        params: params.clone(),
    };
    let mapped =
        response_params::map_openai_params(capabilities, &options, drop_params, bundled).await?;
    let raw = serde_json::to_value(&mapped).map_err(ResponsesError::Encode)?;
    let mut mapped_params: Object = serde_json::from_value(raw).map_err(ResponsesError::Encode)?;
    // The key goes whatever its value and whatever drop_params says, since the endpoint rejects it
    if mapped_params.shift_remove("background").is_some() {
        tracing::warn!(
            "Bedrock Runtime Responses API: dropping unsupported parameter(s) ('background',) that the endpoint rejects."
        );
    }
    drop_unsupported_tool_types(mapped_params)
}

fn drop_unsupported_tool_types(mut mapped_params: Object) -> Result<Object, ResponsesError> {
    let Some(Json::List(tools)) = mapped_params.get("tools") else {
        return Ok(mapped_params);
    };
    let mut entries = Vec::with_capacity(tools.len());
    for tool in tools {
        entries.push(serde_json::to_value(tool).map_err(ResponsesError::Encode)?);
    }
    let supported = SUPPORTED_TOOL_TYPES
        .map(str::to_owned)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let (kept, dropped) = drop_unsupported_tools(&entries, &supported);
    if dropped.is_empty() {
        return Ok(mapped_params);
    }
    tracing::warn!(
        "Bedrock Runtime Responses API: dropping unsupported tool type(s) {} (supported: {}).",
        repr(&dropped)?,
        SUPPORTED_TOOL_TYPES_REPR
    );
    mapped_params.shift_remove("tools");
    if !kept.is_empty() {
        let mut retained = Vec::with_capacity(kept.len());
        for tool in kept {
            retained.push(serde_json::from_value(tool).map_err(ResponsesError::Encode)?);
        }
        mapped_params.insert("tools".to_owned(), Json::List(retained));
    }
    Ok(mapped_params)
}
