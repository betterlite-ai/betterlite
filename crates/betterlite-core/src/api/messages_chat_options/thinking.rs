use betterlite_models::projections::reasoning_effort::{
    ReasoningEffort, ThinkingBudgetThresholds, effort_from_thinking_budget,
};
use betterlite_prompts::arguments::json::{Json, Object};

use crate::api::messages_chat_options::capabilities::{REASONING_EFFORT, declares};
use crate::api::messages_chat_options::context::OptionsContext;
use crate::api::messages_chat_options::error::OptionsError;

const EFFORT_NONE: &str = "none";
const EFFORT_MEDIUM: &str = "medium";
const SUMMARY_DETAILED: &str = "detailed";
const THINKING: &str = "thinking";
const OUTPUT_CONFIG: &str = "output_config";
const THINKING_DISABLED: &str = "disabled";
const THINKING_ENABLED: &str = "enabled";
const THINKING_ADAPTIVE: &str = "adaptive";
const BEDROCK_PREFIXES: [&str; 3] = ["bedrock/", "converse/", "invoke/"];

const fn effort_name(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::Minimal => "minimal",
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => EFFORT_MEDIUM,
        ReasoningEffort::High => "high",
    }
}

fn budget_tokens(thinking: &Object) -> Result<i64, OptionsError> {
    match thinking.get("budget_tokens") {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or(OptionsError::BudgetNotInteger),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(OptionsError::BudgetNotInteger)
        }
    }
}

// The type defaults to disabled, and only the three named kinds say anything; a non-dict thinking asks for no effort at all.
pub(super) fn thinking_effort(
    thinking: &Json,
    thresholds: ThinkingBudgetThresholds,
) -> Result<Option<&'static str>, OptionsError> {
    let Some(fields) = thinking.as_object() else {
        return Ok(None);
    };
    let kind = match fields.get("type") {
        None | Some(Json::Null) => Some(THINKING_DISABLED),
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => None,
    };
    let effort = match kind {
        Some(THINKING_DISABLED) => Some(EFFORT_NONE),
        Some(THINKING_ENABLED) => Some(effort_name(effort_from_thinking_budget(
            budget_tokens(fields)?,
            thresholds,
        ))),
        Some(THINKING_ADAPTIVE) => Some(EFFORT_MEDIUM),
        _ => None,
    };
    Ok(effort)
}

fn declared_effort(output_config: Option<&Json>) -> Result<Option<&str>, OptionsError> {
    let Some(fields) = output_config.and_then(Json::as_object) else {
        return Ok(None);
    };
    match fields.get("effort") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(effort)) => Ok((!effort.is_empty()).then_some(effort.as_str())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(OptionsError::EffortNotText)
        }
    }
}

// The adaptive tier is the only one a caller can set directly, so an effort without adaptive thinking stays behind.
fn adaptive_effort<'request>(
    thinking: &Json,
    output_config: Option<&'request Json>,
) -> Result<Option<&'request str>, OptionsError> {
    let adaptive = thinking
        .as_object()
        .and_then(|fields| fields.get("type"))
        .and_then(Json::as_str)
        == Some(THINKING_ADAPTIVE);
    if !adaptive {
        return Ok(None);
    }
    declared_effort(output_config)
}

// Disabled thinking stays a plain string: there is no trace to summarize, and some providers reject the wrapped dict.
#[must_use]
pub fn summary_wrapped_effort(effort: &str, thinking: &Json, auto_summary: bool) -> Json {
    let fields = thinking.as_object();
    let disabled = fields
        .and_then(|object| object.get("type"))
        .and_then(Json::as_str)
        == Some(THINKING_DISABLED);
    if disabled {
        return Json::Text(effort.to_owned());
    }
    if let Some(summary) = fields
        .and_then(|object| object.get("summary"))
        .filter(|value| value.is_truthy())
    {
        return summary_object(effort, summary);
    }
    if auto_summary {
        return summary_object(effort, &Json::Text(SUMMARY_DETAILED.to_owned()));
    }
    Json::Text(effort.to_owned())
}

fn summary_object(effort: &str, summary: &Json) -> Json {
    let mut wrapped = Object::new();
    wrapped.insert("effort".to_owned(), Json::Text(effort.to_owned()));
    wrapped.insert("summary".to_owned(), summary.clone());
    Json::Object(wrapped)
}

fn forward_bedrock_output_config(output_config: Option<&Json>, kwargs: &mut Object) {
    let Some(fields) = output_config.and_then(Json::as_object) else {
        return;
    };
    let mut effort_config = fields.clone();
    effort_config.shift_remove("format");
    if !effort_config.is_empty() {
        kwargs.insert(OUTPUT_CONFIG.to_owned(), Json::Object(effort_config));
    }
}

// The effort a non-Claude target receives: the caller's adaptive tier when it set one, and the bucketed budget otherwise.
fn wrapped_effort(
    thinking: &Json,
    output_config: Option<&Json>,
    context: &OptionsContext<'_>,
) -> Result<Option<Json>, OptionsError> {
    let declared = adaptive_effort(thinking, output_config)?;
    let effort = match declared {
        Some(effort) => Some(effort),
        None => thinking_effort(thinking, context.thresholds)?,
    };
    let Some(effort) = effort else {
        return Ok(None);
    };
    Ok(Some(summary_wrapped_effort(
        effort,
        thinking,
        context.auto_summary,
    )))
}

// A Claude target keeps thinking verbatim; Bedrock carries the tier as output_config, another provider only when it declares reasoning_effort.
pub fn translate_thinking(
    request: &Object,
    kwargs: &mut Object,
    context: &OptionsContext<'_>,
) -> Result<(), OptionsError> {
    let Some(thinking) = request.get(THINKING).filter(|value| value.is_truthy()) else {
        return Ok(());
    };
    let output_config = request.get(OUTPUT_CONFIG);
    let lowered = context.model.to_lowercase();
    let bedrock_arn = lowered.contains("arn:") && lowered.contains(":bedrock:");
    let claude = lowered.contains("anthropic") || lowered.contains("claude") || bedrock_arn;
    if !claude {
        if let Some(effort) = wrapped_effort(thinking, output_config, context)? {
            kwargs.insert(REASONING_EFFORT.to_owned(), effort);
        }
        return Ok(());
    }
    kwargs.insert(THINKING.to_owned(), thinking.clone());
    let bedrock = BEDROCK_PREFIXES
        .iter()
        .any(|prefix| context.model.starts_with(prefix))
        || bedrock_arn;
    if bedrock {
        forward_bedrock_output_config(output_config, kwargs);
        return Ok(());
    }
    if !declares(context, REASONING_EFFORT) {
        return Ok(());
    }
    if let Some(effort) = adaptive_effort(thinking, output_config)? {
        kwargs.insert(REASONING_EFFORT.to_owned(), Json::Text(effort.to_owned()));
    }
    Ok(())
}
