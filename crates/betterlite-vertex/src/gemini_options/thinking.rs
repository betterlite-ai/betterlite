use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

use crate::gemini_thinking::effort::{map_effort_to_budget, map_effort_to_level};
use crate::gemini_thinking::param::{
    ThinkingParam, map_thinking_param, validate_thinking_conflicts,
};
use crate::model_urls::model_name::is_gemini_3_or_newer;

use super::assembly::OptionsContext;
use super::error::{GeminiOptionsError, decode, encode};

// OpenAI Agents SDK clients send the object form, and only its effort member is read
#[derive(Deserialize)]
struct EffortRequest {
    effort: Option<String>,
}

pub fn map(
    param: &str,
    value: &Json,
    optional: &mut Object,
    context: &OptionsContext<'_>,
) -> Result<(), GeminiOptionsError> {
    if param == "thinking" {
        return map_thinking(value, optional, context);
    }
    map_reasoning_effort(value, optional, context)
}

pub fn map_reasoning_effort(
    value: &Json,
    optional: &mut Object,
    context: &OptionsContext<'_>,
) -> Result<(), GeminiOptionsError> {
    let effort = match value {
        Json::Text(text) => text.clone(),
        Json::Object(_) => match decode::<EffortRequest>(value, "reasoning_effort")?.effort {
            Some(effort) => effort,
            None => return Ok(()),
        },
        Json::Null => return Ok(()),
        Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            return Err(GeminiOptionsError::FieldType {
                param: "reasoning_effort".to_owned(),
                expected: "a string or an object with a string `effort`",
            });
        }
    };
    validate_thinking_conflicts(optional, "reasoning_effort")?;
    let config = if is_gemini_3_or_newer(context.model) {
        map_effort_to_level(&effort, Some(context.model), context.support)?
    } else {
        map_effort_to_budget(&effort, Some(context.model), &context.thinking.budgets)?
    };
    optional.insert("thinkingConfig".to_owned(), encode(config)?);
    Ok(())
}

pub fn map_thinking(
    value: &Json,
    optional: &mut Object,
    context: &OptionsContext<'_>,
) -> Result<(), GeminiOptionsError> {
    let param: ThinkingParam = decode(value, "thinking")?;
    validate_thinking_conflicts(optional, "thinking")?;
    let config = map_thinking_param(
        &param,
        Some(context.model),
        context.thinking,
        context.support,
    );
    optional.insert("thinkingConfig".to_owned(), encode(config)?);
    Ok(())
}
