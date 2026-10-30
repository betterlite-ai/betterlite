use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use self::presence::Field;
pub mod presence;

pub const MAX_ERROR_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderErrorFacts {
    #[serde(alias = "status")]
    pub status_code: Option<u16>,
    pub status_code_is_synthesized: bool,
    pub message: Option<String>,
    pub body: Option<ProviderErrorBody>,
    pub headers: Option<BTreeMap<String, String>>,
    pub response: Option<ResponseFacts>,
    pub litellm_response_headers: Option<BTreeMap<String, String>>,
    pub request: Option<RequestFacts>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderErrorBody {
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub message: Field<String>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub error: Field<Box<Self>>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub code: Field<ErrorCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ErrorCode {
    Number(i64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestFacts {
    pub method: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseFacts {
    pub status_code: u16,
    pub headers: BTreeMap<String, String>,
    pub request: Option<RequestFacts>,
}

pub struct ExceptionCheckers;

impl ExceptionCheckers {
    #[must_use]
    pub fn is_error_str_rate_limit(error: &str, status_code: Option<u16>) -> bool {
        let standalone = error
            .split(|character: char| !(character.is_alphanumeric() || character == '_'))
            .any(|word| word == "429");
        let status_allows_number = status_code.is_none_or(|status| status == 429);
        let lower = error.to_lowercase();
        let phrase = lower.split("rate").skip(1).any(|suffix| {
            suffix
                .trim_start_matches(|character: char| {
                    character.is_whitespace()
                        || matches!(character, '_' | '-' | '\u{1c}'..='\u{1f}')
                })
                .starts_with("limit")
        });
        (standalone && status_allows_number)
            || phrase
            || lower.contains("service tier capacity exceeded")
    }

    #[must_use]
    pub fn context_window(error: &str) -> bool {
        let lower = error.to_lowercase();
        if lower.contains("string_above_max_length")
            || (lower.contains("invalid 'user'") && lower.contains("string too long"))
        {
            return false;
        }
        [
            "exceed context limit",
            "this model's maximum context length is",
            "string too long. expected a string with maximum length",
            "model's maximum context limit",
            "is longer than the model's context length",
            "input tokens exceed the configured limit",
            "`inputs` tokens + `max_new_tokens` must be",
            "exceeds the available context size",
            "exceeds the maximum number of tokens allowed",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
            || (lower.contains("current length is") && lower.contains("while limit is"))
            || (lower.contains("maximum input length is") && lower.contains("tokens"))
    }

    #[must_use]
    pub fn azure_policy(error: &str) -> bool {
        let lower = error.to_lowercase();
        [
            "content_policy_violation",
            "responsibleaipolicyviolation",
            "the response was filtered due to the prompt triggering azure openai's content management",
            "your task failed as a result of our safety system",
            "the model produced invalid content",
            "content_filter_policy",
            "your request was rejected as a result of our safety system",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
    }
}

#[must_use]
pub fn litellm_proxy_response(
    error: &ProviderErrorFacts,
    custom_provider: &str,
) -> Option<ResponseFacts> {
    let response = error.response.as_ref()?;
    if custom_provider != "litellm_proxy" || !response.headers.is_empty() {
        return Some(response.clone());
    }
    error
        .headers
        .as_ref()
        .filter(|headers| !headers.is_empty())
        .map_or_else(
            || Some(response.clone()),
            |headers| {
                Some(ResponseFacts {
                    status_code: response.status_code,
                    headers: headers.clone(),
                    request: error.request.clone(),
                })
            },
        )
}

#[must_use]
pub fn bounded_text(text: &str) -> String {
    let mut bytes = 0_usize;
    let mut bounded = String::new();
    for character in text.chars() {
        let Some(next) = bytes.checked_add(character.len_utf8()) else {
            break;
        };
        if next > MAX_ERROR_TEXT_BYTES {
            break;
        }
        bounded.push(character);
        bytes = next;
    }
    bounded
}

#[must_use]
pub fn bounded_body(body: &ProviderErrorBody) -> Option<ProviderErrorBody> {
    let encoded = serde_json::to_vec(body).ok()?;
    if encoded.len() > MAX_BODY_BYTES {
        return None;
    }
    Some(body.clone())
}

mod tests;
