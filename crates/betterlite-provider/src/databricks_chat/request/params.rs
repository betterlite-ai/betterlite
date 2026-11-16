use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::params::DEFAULT_MAX_TOKENS;
use betterlite_core::api::messages_native_transform::capabilities::{
    ADAPTIVE_THINKING, LegacyShapes, ThinkingCapabilities,
};
use betterlite_models::decisions::Json;
use betterlite_openai::base_helpers::{add_thinking_max_tokens, thinking_request};
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};

use super::CLAUDE;
use super::SUPPORTED_OPENAI_PARAMS;
use super::tools;
use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::wire::{Params, convert};

pub(crate) const PROVIDER: &str = "databricks";
const ANTHROPIC_PAYLOAD: &str = "supports_anthropic_thinking_payload";

// LiteLLM's map_openai_params, in the order the Databricks transform applies its steps
pub async fn map_openai_params<Capabilities: ThinkingCapabilities + Send>(
    non_default_params: &Object,
    optional_params: &mut Object,
    model: &str,
    _drop_params: bool,
    capabilities: &mut Capabilities,
) -> Result<(), DatabricksChatError> {
    let non_default: Params = convert(non_default_params)?;
    let mut mapped: Params = convert(optional_params)?;
    let is_thinking_enabled = thinking_enabled(&non_default);
    apply_parent(&non_default, &mut mapped);
    if model.contains(CLAUDE) {
        translate_legacy(model, &mut mapped, capabilities).await?;
    }
    if let Some(tools_value) = mapped.get("tools") {
        let given: PromptJson = convert(tools_value)?;
        let converted = convert(&tools::map_tools_for_model(model, &given)?)?;
        mapped.insert("tools".to_owned(), converted);
    }
    if let Some(limit) = non_default.get("max_completion_tokens") {
        mapped.insert("max_tokens".to_owned(), limit.clone());
        mapped.shift_remove("max_completion_tokens");
    }
    if model.contains(CLAUDE)
        && let Some(value) = non_default.get("response_format")
    {
        apply_response_format(&mut mapped, value, is_thinking_enabled)?;
    }
    if let Some(value) = non_default.get("reasoning_effort")
        && uses_thinking_param(model, capabilities).await
    {
        apply_reasoning_effort(model, value, &mut mapped, capabilities).await?;
        mapped.shift_remove("reasoning_effort");
    }
    update_thinking_tokens(&non_default, &mut mapped)?;
    *optional_params = convert(&mapped)?;
    Ok(())
}

// The OpenAI base policy copies every non-default parameter the provider lists as supported
fn apply_parent(non_default: &Params, mapped: &mut Params) {
    for (name, value) in non_default {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            mapped.insert(name.clone(), value.clone());
        }
    }
}

fn thinking_enabled(non_default: &Params) -> bool {
    let Ok(requested) = serde_json::to_value(non_default) else {
        return false;
    };
    let Some(fields) = requested.as_object() else {
        return false;
    };
    thinking_request(fields).thinking
}

// _uses_anthropic_thinking_param lowercases the model and swaps "." for "-" before the check
async fn uses_thinking_param<Capabilities: ThinkingCapabilities + Send>(
    model: &str,
    capabilities: &mut Capabilities,
) -> bool {
    let normalized = model.to_lowercase().replace('.', "-");
    if normalized.contains(CLAUDE) {
        return true;
    }
    capabilities
        .supports(&normalized, ANTHROPIC_PAYLOAD, PROVIDER)
        .await
}

// translate_legacy_thinking_for_adaptive_model writes back only the keys it changed
async fn translate_legacy<Capabilities: ThinkingCapabilities + Send>(
    model: &str,
    mapped: &mut Params,
    capabilities: &mut Capabilities,
) -> Result<(), DatabricksChatError> {
    let before_thinking = mapped.get("thinking").cloned();
    let before_output_config = mapped.get("output_config").cloned();
    let mut params = OptionalParams {
        reasoning_effort: None,
        thinking: before_thinking.clone(),
        output_config: before_output_config.clone(),
    };
    let mut shapes = LegacyShapes {
        model,
        custom_llm_provider: PROVIDER,
        params: &mut params,
    };
    capabilities
        .translate_legacy_thinking(&mut shapes)
        .await
        .map_err(DatabricksChatError::Thinking)?;
    if params.thinking != before_thinking {
        mapped.shift_remove("thinking");
        if let Some(value) = &params.thinking {
            mapped.insert("thinking".to_owned(), value.clone());
        }
    }
    if params.output_config != before_output_config
        && let Some(value) = &params.output_config
    {
        mapped.insert("output_config".to_owned(), value.clone());
    }
    Ok(())
}

// map_response_format_to_databricks_tool and the json_mode tool_choice it installs
fn apply_response_format(
    mapped: &mut Params,
    value: &Json,
    is_thinking_enabled: bool,
) -> Result<(), DatabricksChatError> {
    let format: PromptJson = convert(value)?;
    if let Some(tool) = tools::response_format_tool(&format)? {
        tools::append_wire_tool(mapped, convert(&tool)?)?;
        mapped.insert("json_mode".to_owned(), Json::Bool(true));
        if !is_thinking_enabled {
            mapped.insert(
                "tool_choice".to_owned(),
                convert(&tools::json_tool_choice())?,
            );
        }
    }
    mapped.shift_remove("response_format");
    Ok(())
}

// _map_reasoning_effort then the adaptive output_config.effort overlay, with the model's view
async fn apply_reasoning_effort<Capabilities: ThinkingCapabilities + Send>(
    model: &str,
    value: &Json,
    mapped: &mut Params,
    capabilities: &mut Capabilities,
) -> Result<(), DatabricksChatError> {
    let effort = match value {
        Json::Null => None,
        Json::Text(effort) => Some(effort.as_str()),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            return Err(DatabricksChatError::InvalidEffort {
                value: serde_json::to_string(value).map_err(DatabricksChatError::Encode)?,
            });
        }
    };
    let mapped_thinking = match effort {
        Some(requested) => capabilities
            .map_effort(model, requested, PROVIDER)
            .await
            .map_err(DatabricksChatError::Reasoning)?,
        None => None,
    };
    let Some(thinking) = mapped_thinking else {
        mapped.shift_remove("thinking");
        mapped.shift_remove("output_config");
        return Ok(());
    };
    mapped.insert("thinking".to_owned(), Json::Object(thinking));
    if !capabilities
        .supports(model, ADAPTIVE_THINKING, PROVIDER)
        .await
    {
        return Ok(());
    }
    let Some(level) = effort.and_then(effort_level) else {
        return Err(DatabricksChatError::InvalidEffort {
            value: effort.unwrap_or_default().to_owned(),
        });
    };
    let output_config = Params::from([("effort".to_owned(), Json::Text(level.to_owned()))]);
    mapped.insert("output_config".to_owned(), Json::Object(output_config));
    Ok(())
}

// REASONING_EFFORT_TO_OUTPUT_CONFIG_EFFORT
fn effort_level(effort: &str) -> Option<&'static str> {
    match effort {
        "minimal" | "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" => Some("xhigh"),
        "max" => Some("max"),
        _ => None,
    }
}

// update_optional_params_with_thinking_tokens, through B2202's checked budget arithmetic
fn update_thinking_tokens(
    non_default: &Params,
    mapped: &mut Params,
) -> Result<(), DatabricksChatError> {
    let requested = convert(non_default)?;
    let mut options = convert(&*mapped)?;
    add_thinking_max_tokens(&requested, &mut options, DEFAULT_MAX_TOKENS)
        .map_err(DatabricksChatError::Base)?;
    *mapped = convert(&options)?;
    Ok(())
}
