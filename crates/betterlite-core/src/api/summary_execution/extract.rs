use betterlite_prompts::arguments::json::Json;
use num_traits::cast::cast;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Number;

// the typed record a completion port returns: _extract_response_text and _extract_usage read only these fields
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct CompletionAnswer {
    #[serde(default, deserialize_with = "nullable_choices")]
    pub choices: Vec<CompletionChoice>,
    #[serde(default)]
    pub usage: Option<CompletionUsage>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct CompletionChoice {
    #[serde(default)]
    pub message: Option<CompletionMessage>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct CompletionMessage {
    // a string, a list of content parts, or anything else, which reads as no text
    #[serde(default)]
    pub content: Option<Json>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct CompletionUsage {
    #[serde(default)]
    pub prompt_tokens: Option<Json>,
    #[serde(default)]
    pub completion_tokens: Option<Json>,
}

// getattr(response, "choices", None) reads a null the same way as a missing field
fn nullable_choices<'de, D>(deserializer: D) -> Result<Vec<CompletionChoice>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<Vec<CompletionChoice>>::deserialize(deserializer).map(Option::unwrap_or_default)
}

// the errors _extract_response_text's TypeError and _extract_usage's ValueError stand for
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExtractionError {
    #[error("a text content part carries a value that is not a string")]
    NonTextPart,
    #[error("a usage token count is not an integer")]
    TokenNotAnInteger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SummaryUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub fn extract_response_text(answer: &CompletionAnswer) -> Result<Option<String>, ExtractionError> {
    let Some(content) = content_of(answer) else {
        return Ok(None);
    };
    match content {
        Json::Text(text) => Ok(Some(text.clone())),
        Json::List(parts) => text_parts(parts),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => Ok(None),
    }
}

fn content_of(answer: &CompletionAnswer) -> Option<&Json> {
    let choice = answer.choices.first()?;
    let message = choice.message.as_ref()?;
    message.content.as_ref()
}

// only dict parts whose type is "text" join, and a missing text reads as the empty string
fn text_parts(parts: &[Json]) -> Result<Option<String>, ExtractionError> {
    let mut joined = String::new();
    for part in parts {
        let Some(fields) = part.as_object() else {
            continue;
        };
        if fields.get("type").and_then(Json::as_str) != Some("text") {
            continue;
        }
        match fields.get("text") {
            None => {}
            Some(Json::Text(text)) => joined.push_str(text),
            Some(_) => return Err(ExtractionError::NonTextPart),
        }
    }
    Ok((!joined.is_empty()).then_some(joined))
}

pub fn extract_usage(answer: &CompletionAnswer) -> Result<SummaryUsage, ExtractionError> {
    let Some(usage) = answer.usage.as_ref() else {
        return Ok(SummaryUsage {
            input_tokens: 0,
            output_tokens: 0,
        });
    };
    Ok(SummaryUsage {
        input_tokens: token_count(usage.prompt_tokens.as_ref())?,
        output_tokens: token_count(usage.completion_tokens.as_ref())?,
    })
}

// int(value or 0): null and an empty string count as zero, a numeric string parses, anything else raises
fn token_count(value: Option<&Json>) -> Result<u64, ExtractionError> {
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Bool(flag)) => Ok(u64::from(*flag)),
        Some(Json::Number(number)) => number_tokens(number),
        Some(Json::Text(text)) if text.is_empty() => Ok(0),
        Some(Json::Text(text)) => text
            .trim()
            .parse::<u64>()
            .map_err(|_not_integer| ExtractionError::TokenNotAnInteger),
        Some(Json::List(_) | Json::Object(_)) => Err(ExtractionError::TokenNotAnInteger),
    }
}

fn number_tokens(number: &Number) -> Result<u64, ExtractionError> {
    if let Some(tokens) = number.as_u64() {
        return Ok(tokens);
    }
    if let Some(tokens) = number.as_i64() {
        return u64::try_from(tokens).map_err(|_negative| ExtractionError::TokenNotAnInteger);
    }
    let Some(float) = number.as_f64().filter(|value| *value >= 0.0_f64) else {
        return Err(ExtractionError::TokenNotAnInteger);
    };
    cast::<f64, u64>(float.trunc()).ok_or(ExtractionError::TokenNotAnInteger)
}
