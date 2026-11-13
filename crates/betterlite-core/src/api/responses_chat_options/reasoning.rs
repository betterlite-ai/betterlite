use betterlite_config::sections::settings::JsonValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::responses_bridge::{BridgeRequest, ReasoningEffort};

use super::capabilities::CompletionBridgeProbe;
use super::error::ChatOptionsError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReasoningChatForm {
    pub effort: Option<String>,
    pub summary: Option<String>,
}

pub(crate) struct BridgeInputs<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub web_search_options: Option<&'request JsonValue>,
    pub tools: &'request [JsonValue],
    pub api_base: Option<&'request str>,
    pub route_all_openai_to_responses: bool,
}

// summary only rides a chat request that litellm.completion sends back through the Responses API
pub(crate) fn reasoning_for_chat(
    reasoning: Option<&Json>,
    bridge: &BridgeInputs<'_>,
    probe: &dyn CompletionBridgeProbe,
) -> Result<ReasoningChatForm, ChatOptionsError> {
    let Some(fields) = reasoning_fields(reasoning)? else {
        return Ok(ReasoningChatForm::default());
    };
    let Some(summary_text) = fields.summary else {
        return Ok(fields);
    };
    let effort_value = fields.effort.clone().map(ReasoningEffort::Text);
    let summary_value = JsonValue::String(summary_text.clone());
    let request = BridgeRequest {
        model: bridge.model,
        custom_llm_provider: bridge.custom_llm_provider.unwrap_or(""),
        web_search_options: bridge.web_search_options,
        tools: bridge.tools,
        reasoning_effort: effort_value.as_ref(),
        reasoning_summary: Some(&summary_value),
        api_base: bridge.api_base,
        route_all_openai_to_responses: bridge.route_all_openai_to_responses,
    };
    let bridged = match probe.bridges_back(&request) {
        Ok(bridged) => bridged,
        Err(error) => {
            tracing::debug!(
                error = %error,
                "responses bridge: reasoning effort mode check failed: {error}"
            );
            false
        }
    };
    Ok(ReasoningChatForm {
        effort: fields.effort,
        summary: bridged.then_some(summary_text),
    })
}

// a null, an empty string and an object without effort or summary all mean no reasoning params
fn reasoning_fields(
    reasoning: Option<&Json>,
) -> Result<Option<ReasoningChatForm>, ChatOptionsError> {
    match reasoning {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(text_reasoning(text)),
        Some(Json::Object(fields)) => object_reasoning(fields),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            Err(ChatOptionsError::WrongType { field: "reasoning" })
        }
    }
}

fn text_reasoning(text: &str) -> Option<ReasoningChatForm> {
    if text.is_empty() {
        return None;
    }
    Some(ReasoningChatForm {
        effort: Some(text.to_owned()),
        summary: None,
    })
}

fn object_reasoning(
    fields: &IndexMap<String, Json>,
) -> Result<Option<ReasoningChatForm>, ChatOptionsError> {
    let effort = optional_text(fields.get("effort"), "reasoning.effort")?;
    let summary = optional_text(fields.get("summary"), "reasoning.summary")?;
    if effort.is_none() && summary.is_none() {
        return Ok(None);
    }
    Ok(Some(ReasoningChatForm { effort, summary }))
}

fn optional_text(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<String>, ChatOptionsError> {
    match value {
        Some(Json::Text(text)) => Ok(Some(text.to_owned())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ChatOptionsError::WrongType { field })
        }
        None | Some(Json::Null) => Ok(None),
    }
}
