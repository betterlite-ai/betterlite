use betterlite_models::metadata::levels::ReasoningEffort;
use betterlite_models::projections::reasoning_effort::{
    ReasoningEffort as BudgetEffort, effort_from_thinking_budget,
};
use betterlite_prompts::arguments::json::Json;
use rust_decimal::prelude::ToPrimitive as _;

use super::error::MessagesResponsesOutputError;
use super::outputs::{
    CompactionKind, ForcedFunction, ForcedFunctionKind, ResponsesContextManagement,
    ResponsesReasoning, ResponsesText, ResponsesTextFormat, ResponsesTool, ResponsesToolChoice,
    StructuredOutputKind, ToolChoiceMode,
};
use super::records::{
    AnthropicRequestRecord, ContextManagementRecord, OutputConfigRecord, OutputFormatRecord,
    SettingsRecord, ThinkingRecord, ToolChoiceKind, ToolChoiceRecord, ToolRecord,
};

const COMPACT_EDIT_TYPE: &str = "compact_20260112";
const JSON_SCHEMA_TYPE: &str = "json_schema";
const STRUCTURED_OUTPUT_NAME: &str = "structured_output";
const AUTO_SUMMARY: &str = "detailed";

#[must_use]
pub fn translate_tools(tools: &[ToolRecord]) -> Vec<ResponsesTool> {
    let mut translated = Vec::with_capacity(tools.len());
    for tool in tools {
        if tool.r#type.starts_with("web_search") || tool.name == "web_search" {
            translated.push(ResponsesTool::WebSearchPreview);
            continue;
        }
        translated.push(ResponsesTool::Function {
            name: tool.name.clone(),
            strict: tool.strict,
            description: tool.description.clone(),
            parameters: tool.input_schema.clone(),
        });
    }
    translated
}

#[must_use]
pub fn translate_tool_choice(choice: &ToolChoiceRecord) -> ResponsesToolChoice {
    match choice.kind {
        ToolChoiceKind::Any => ResponsesToolChoice::Mode(ToolChoiceMode::Required),
        ToolChoiceKind::Tool => ResponsesToolChoice::Function(ForcedFunction {
            r#type: ForcedFunctionKind::Function,
            name: choice.name.clone(),
        }),
        ToolChoiceKind::None => ResponsesToolChoice::Mode(ToolChoiceMode::None),
        ToolChoiceKind::Auto => ResponsesToolChoice::Mode(ToolChoiceMode::Auto),
    }
}

pub fn translate_context_management(
    management: &ContextManagementRecord,
) -> Result<Option<Vec<ResponsesContextManagement>>, MessagesResponsesOutputError> {
    let Some(Json::List(edits)) = management.edits.as_ref() else {
        return Ok(None);
    };
    let mut translated = Vec::new();
    for edit in edits {
        let Some(fields) = edit.as_object() else {
            continue;
        };
        if fields.get("type").and_then(Json::as_str) != Some(COMPACT_EDIT_TYPE) {
            continue;
        }
        let compact_threshold = fields
            .get("trigger")
            .and_then(trigger_threshold)
            .transpose()?;
        translated.push(ResponsesContextManagement {
            r#type: CompactionKind::Compaction,
            compact_threshold,
        });
    }
    Ok((!translated.is_empty()).then_some(translated))
}

fn trigger_threshold(trigger: &Json) -> Option<Result<i64, MessagesResponsesOutputError>> {
    let Json::Object(fields) = trigger else {
        return None;
    };
    let value = fields.get("value")?;
    if matches!(value, Json::Null) {
        return None;
    }
    Some(integer_value(value))
}

fn integer_value(value: &Json) -> Result<i64, MessagesResponsesOutputError> {
    match value {
        Json::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().and_then(|float| float.trunc().to_i64()))
            .ok_or(MessagesResponsesOutputError::ContextThreshold),
        Json::Text(text) => text
            .trim()
            .parse::<i64>()
            .map_err(|_source| MessagesResponsesOutputError::ContextThreshold),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => {
            Err(MessagesResponsesOutputError::ContextThreshold)
        }
    }
}

pub fn translate_thinking(
    thinking: &ThinkingRecord,
    output_config: Option<&OutputConfigRecord>,
    settings: &SettingsRecord,
) -> Result<Option<ResponsesReasoning>, MessagesResponsesOutputError> {
    let effort = match thinking.r#type.as_ref().and_then(Json::as_str) {
        Some("adaptive") => adaptive_effort(output_config)?,
        Some("enabled") => responses_effort(effort_from_thinking_budget(
            thinking.budget_tokens.as_ref().map_or(0, budget_tokens),
            settings.budget_thresholds,
        )),
        Some(_) | None => return Ok(None),
    };
    let summary = thinking
        .summary
        .as_deref()
        .filter(|summary| !summary.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            settings
                .reasoning_auto_summary
                .then(|| AUTO_SUMMARY.to_owned())
        });
    Ok(Some(ResponsesReasoning { effort, summary }))
}

fn adaptive_effort(
    output_config: Option<&OutputConfigRecord>,
) -> Result<ReasoningEffort, MessagesResponsesOutputError> {
    let effort = output_config
        .and_then(|config| config.effort.as_deref())
        .filter(|effort| !effort.is_empty());
    match effort {
        Some(effort) => requested_effort(effort),
        None => Ok(ReasoningEffort::Medium),
    }
}

// output_config.effort is the declared Literal the pinned type accepts, so anything else is refused
fn requested_effort(effort: &str) -> Result<ReasoningEffort, MessagesResponsesOutputError> {
    match effort {
        "low" => Ok(ReasoningEffort::Low),
        "medium" => Ok(ReasoningEffort::Medium),
        "high" => Ok(ReasoningEffort::High),
        "xhigh" => Ok(ReasoningEffort::Xhigh),
        "max" => Ok(ReasoningEffort::Max),
        _ => Err(MessagesResponsesOutputError::Effort {
            effort: effort.to_owned(),
        }),
    }
}

fn budget_tokens(value: &Json) -> i64 {
    if matches!(value, Json::Number(_)) {
        return integer_value(value).unwrap_or(0);
    }
    0
}

const fn responses_effort(effort: BudgetEffort) -> ReasoningEffort {
    match effort {
        BudgetEffort::Minimal => ReasoningEffort::Minimal,
        BudgetEffort::Low => ReasoningEffort::Low,
        BudgetEffort::Medium => ReasoningEffort::Medium,
        BudgetEffort::High => ReasoningEffort::High,
    }
}

pub fn translate_output_format(request: &AnthropicRequestRecord) -> Option<ResponsesText> {
    let format = match &request.output_format {
        Some(format) => format,
        None => request.output_config.as_ref()?.format.as_ref()?,
    };
    if format.r#type.as_deref() != Some(JSON_SCHEMA_TYPE) {
        return None;
    }
    let schema = format.schema.as_ref().filter(|schema| !schema.is_empty())?;
    Some(ResponsesText {
        format: ResponsesTextFormat {
            r#type: StructuredOutputKind::JsonSchema,
            name: STRUCTURED_OUTPUT_NAME,
            schema: schema.clone(),
            strict: format.strict,
        },
    })
}
