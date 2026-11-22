use betterlite_prompts::arguments::json::Json;

use crate::api::messages_context_clear::constants::{
    COMPACT_DEFAULT_TRIGGER_TOKENS, COMPACT_EDIT_TYPE, COMPACT_MIN_TRIGGER_TOKENS,
};
use crate::api::messages_context_clear::error::ContextManagementError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerTokens {
    pub tokens: u64,
    pub warnings: Vec<String>,
}

// Python's bool is an int, every negative is below the minimum, and an integer beyond u64 is a threshold no count reaches
fn integer_tokens(value: &Json) -> Option<u64> {
    let literal = match value {
        Json::Bool(flag) => return Some(u64::from(*flag)),
        Json::Number(number) => number.to_string(),
        Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => return None,
    };
    let (negative, digits) = literal
        .strip_prefix('-')
        .map_or((false, literal.as_str()), |digits| (true, digits));
    if digits.is_empty() || !digits.bytes().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    Some(if negative {
        0_u64
    } else {
        digits.parse::<u64>().unwrap_or(u64::MAX)
    })
}

// a null type reads as absent, and any type that is not a string has no name to put in the warning
fn type_warning(kind: Option<&Json>) -> Option<String> {
    match kind {
        Some(Json::Text(name)) if name != "input_tokens" => Some(format!(
            "unsupported_trigger_type_{name}_using_input_tokens"
        )),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Some("unsupported_trigger_type_using_input_tokens".to_owned())
        }
        Some(Json::Null | Json::Text(_)) | None => None,
    }
}

pub fn resolve_trigger_tokens(
    trigger: Option<&Json>,
) -> Result<TriggerTokens, ContextManagementError> {
    let fields = match trigger.filter(|value| value.is_truthy()) {
        None => {
            return Ok(TriggerTokens {
                tokens: COMPACT_DEFAULT_TRIGGER_TOKENS,
                warnings: Vec::new(),
            });
        }
        Some(Json::Object(fields)) => fields,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Ok(TriggerTokens {
                tokens: COMPACT_DEFAULT_TRIGGER_TOKENS,
                warnings: vec!["trigger_not_a_dict_using_default".to_owned()],
            });
        }
    };
    let mut warnings: Vec<String> = type_warning(fields.get("type")).into_iter().collect();
    let tokens = match fields.get("value").filter(|value| **value != Json::Null) {
        None => COMPACT_DEFAULT_TRIGGER_TOKENS,
        Some(value) => match integer_tokens(value) {
            Some(tokens) if tokens < COMPACT_MIN_TRIGGER_TOKENS => {
                return Err(ContextManagementError {
                    status_code: 400,
                    message: format!(
                        "context_management.{COMPACT_EDIT_TYPE}.trigger.value must be at least {COMPACT_MIN_TRIGGER_TOKENS} tokens"
                    ),
                });
            }
            Some(tokens) => tokens,
            None => {
                warnings.push("trigger_value_not_int_using_default".to_owned());
                COMPACT_DEFAULT_TRIGGER_TOKENS
            }
        },
    };
    Ok(TriggerTokens { tokens, warnings })
}
