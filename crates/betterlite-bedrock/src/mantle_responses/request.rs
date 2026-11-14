use betterlite_core::api::codex_history_tools::{
    HoistedAdditionalTools, hoist_additional_tools, normalize_codex_input_items,
};
use betterlite_openai::response_input::{
    ResponsesInput, transform_responses_request as openai_request,
};
use betterlite_prompts::arguments::json::{Json, Object};

use crate::mantle_responses::config::CUSTOM_LLM_PROVIDER;
use crate::mantle_responses::error::MantleResponsesError;
use crate::mantle_responses::repr::quoted_list;
use crate::mantle_responses::tools::filter_unsupported_tools;

pub fn transform_responses_request(
    model: &str,
    input: &ResponsesInput,
    params: &Object,
    model_info: Option<&Object>,
) -> Result<Object, MantleResponsesError> {
    let raw_input = serde_json::to_value(input).map_err(MantleResponsesError::Encode)?;
    let mut existing = Vec::new();
    if let Some(Json::List(entries)) = params.get("tools") {
        for tool in entries {
            existing.push(serde_json::to_value(tool).map_err(MantleResponsesError::Encode)?);
        }
    }
    let hoisted = hoist_additional_tools(&raw_input, Some(existing.as_slice()));
    let (normalized, rewritten) = normalize_codex_input_items(&hoisted.input);
    if !rewritten.is_empty() {
        tracing::warn!(
            "Bedrock Mantle Responses API: rewrote Codex input item type(s) {} that Mantle rejects.",
            quoted_list(&rewritten)
        );
    }
    if let Some(message) = &hoisted.debug {
        tracing::debug!("{message}");
    }
    let selected = if hoisted.hoisted.is_empty() {
        params.clone()
    } else {
        params_with_hoisted_tools(params, &hoisted)?
    };
    let request_input: ResponsesInput =
        serde_json::from_value(normalized).map_err(MantleResponsesError::Encode)?;
    openai_request(
        model,
        &request_input,
        &selected,
        CUSTOM_LLM_PROVIDER,
        model_info,
    )
    .map_err(MantleResponsesError::Input)
}

fn params_with_hoisted_tools(
    params: &Object,
    hoisted: &HoistedAdditionalTools,
) -> Result<Object, MantleResponsesError> {
    let mut listed = Vec::with_capacity(hoisted.tools.len());
    for tool in &hoisted.tools {
        listed.push(serde_json::from_value(tool.clone()).map_err(MantleResponsesError::Encode)?);
    }
    let filtered = filter_unsupported_tools(&listed)?;
    let mut updated = params.clone();
    if filtered.is_empty() {
        updated.shift_remove("tools");
    } else {
        updated.insert("tools".to_owned(), Json::List(filtered));
    }
    Ok(updated)
}
