use betterlite_prompts::arguments::json::{Json, Object};

use crate::gemini_tools::options::map_service_tier;

use super::error::GeminiOptionsError;
use super::generation;
use super::params::supports_penalty_parameters;

pub fn map(
    param: &str,
    value: &Json,
    optional: &mut Object,
    model: &str,
    warned: &mut bool,
) -> Result<(), GeminiOptionsError> {
    match param {
        "temperature" | "top_p" | "top_k" => {
            generation::check_sampling(model, param, value, warned)?;
            optional.insert(param.to_owned(), value.clone());
        }
        "n" | "max_tokens" | "max_completion_tokens" | "seed" | "logprobs" | "top_logprobs" => {
            map_integer_like(param, value, optional)?;
        }
        "stop" => map_stop(value, optional)?,
        "frequency_penalty" | "presence_penalty" => map_penalty(param, value, optional, model)?,
        "service_tier" => map_service_tier_value(value, optional)?,
        "stream" => map_stream(value, optional)?,
        _ => {}
    }
    Ok(())
}

fn map_integer_like(
    param: &str,
    value: &Json,
    optional: &mut Object,
) -> Result<(), GeminiOptionsError> {
    let (target, expected) = match param {
        "n" => ("candidate_count", "an integer"),
        "max_tokens" | "max_completion_tokens" => ("max_output_tokens", "an integer"),
        "logprobs" => ("responseLogprobs", "a boolean"),
        "top_logprobs" => ("logprobs", "an integer"),
        "seed" => ("seed", "an integer"),
        _ => return Ok(()),
    };
    let matched = if param == "logprobs" {
        matches!(value, Json::Bool(_))
    } else {
        matches!(value, Json::Number(number) if number.as_i64().is_some())
    };
    if !matched {
        return Err(GeminiOptionsError::FieldType {
            param: param.to_owned(),
            expected,
        });
    }
    optional.insert(target.to_owned(), value.clone());
    Ok(())
}

fn map_stop(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    match value {
        Json::Text(text) => {
            optional.insert(
                "stop_sequences".to_owned(),
                Json::List(vec![Json::Text(text.clone())]),
            );
        }
        Json::List(items) if items.iter().all(|item| matches!(item, Json::Text(_))) => {
            optional.insert("stop_sequences".to_owned(), value.clone());
        }
        Json::Null => {}
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            return Err(GeminiOptionsError::FieldType {
                param: "stop".to_owned(),
                expected: "a string or a list of strings",
            });
        }
    }
    Ok(())
}

fn map_penalty(
    param: &str,
    value: &Json,
    optional: &mut Object,
    model: &str,
) -> Result<(), GeminiOptionsError> {
    if !matches!(value, Json::Number(_)) {
        return Err(GeminiOptionsError::FieldType {
            param: param.to_owned(),
            expected: "a number",
        });
    }
    if supports_penalty_parameters(model) {
        optional.insert(param.to_owned(), value.clone());
    }
    Ok(())
}

fn map_service_tier_value(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    match value {
        Json::Text(text) => {
            optional.insert(
                "service_tier".to_owned(),
                Json::Text(map_service_tier(text)),
            );
            Ok(())
        }
        Json::Null => Ok(()),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(GeminiOptionsError::FieldType {
                param: "service_tier".to_owned(),
                expected: "a string",
            })
        }
    }
}

fn map_stream(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    match value {
        Json::Bool(true) => {
            optional.insert("stream".to_owned(), Json::Bool(true));
            Ok(())
        }
        Json::Bool(false) | Json::Null => Ok(()),
        Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Err(GeminiOptionsError::FieldType {
                param: "stream".to_owned(),
                expected: "a boolean",
            })
        }
    }
}
