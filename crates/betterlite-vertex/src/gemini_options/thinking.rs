use betterlite_prompts::arguments::json::{Json, Object};

use crate::gemini_thinking::effort::{map_effort_to_budget, map_effort_to_level};
use crate::gemini_thinking::param::{
    ThinkingParam, map_thinking_param, validate_thinking_conflicts,
};
use crate::model_urls::model_name::is_gemini_3_or_newer;

use super::assembly::OptionsContext;
use super::convert;
use super::error::GeminiOptionsError;

// OpenAI Agents SDK clients send the object form, and only its effort member is read
pub fn map_reasoning_effort(
    value: &Json,
    optional: &mut Object,
    context: &OptionsContext<'_>,
) -> Result<(), GeminiOptionsError> {
    let effort = match value {
        Json::Text(effort) => effort.as_str(),
        Json::Object(fields) => match fields.get("effort") {
            None | Some(Json::Null) => return Ok(()),
            Some(Json::Text(effort)) => effort.as_str(),
            Some(_) => {
                return Err(GeminiOptionsError::FieldType {
                    param: "reasoning_effort".to_owned(),
                    expected: "a string or an object with a string `effort`",
                });
            }
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
        map_effort_to_level(effort, Some(context.model), context.support)?
    } else {
        map_effort_to_budget(effort, Some(context.model), &context.thinking.budgets)?
    };
    optional.insert("thinkingConfig".to_owned(), convert::encode(config)?);
    Ok(())
}

pub fn map_thinking(
    value: &Json,
    optional: &mut Object,
    context: &OptionsContext<'_>,
) -> Result<(), GeminiOptionsError> {
    match value {
        Json::Object(_) => {}
        Json::Null => return Ok(()),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(GeminiOptionsError::FieldType {
                param: "thinking".to_owned(),
                expected: "an object",
            });
        }
    }
    let param: ThinkingParam = convert::decode(value, "thinking")?;
    validate_thinking_conflicts(optional, "thinking")?;
    let config = map_thinking_param(
        &param,
        Some(context.model),
        context.thinking,
        context.support,
    );
    optional.insert("thinkingConfig".to_owned(), convert::encode(config)?);
    Ok(())
}
