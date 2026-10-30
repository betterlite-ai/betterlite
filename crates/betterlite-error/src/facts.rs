use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
    pub response_headers: Option<BTreeMap<String, String>>,
    pub litellm_response_headers: Option<BTreeMap<String, String>>,
    pub request: Option<RequestFacts>,
}

impl ProviderErrorFacts {
    #[must_use]
    pub const fn empty() -> Self {
        let facts = Self {
            status_code: None,
            status_code_is_synthesized: false,
            message: None,
            body: None,
            headers: None,
            response_headers: None,
            litellm_response_headers: None,
            request: None,
        };
        if facts.status_code.is_none() {
            return facts;
        }
        Self {
            status_code: None,
            status_code_is_synthesized: false,
            message: None,
            body: None,
            headers: None,
            response_headers: None,
            litellm_response_headers: None,
            request: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderErrorBody {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<Box<Self>>,
    #[serde(default)]
    pub code: Option<ErrorCode>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyResponseFacts {
    pub status_code: u16,
    pub headers: BTreeMap<String, String>,
    pub request: Option<RequestFacts>,
}

pub struct ExceptionCheckers;

impl ExceptionCheckers {
    #[must_use]
    pub fn is_error_str_rate_limit(error: &str, status_code: Option<u16>) -> bool {
        let standalone = error.char_indices().any(|(index, current_character)| {
            if current_character != '4' {
                return false;
            }
            let Some(candidate) = error.get(index..index.saturating_add(3)) else {
                return false;
            };
            if candidate != "429" {
                return false;
            }
            let before_is_word = index
                .checked_sub(1)
                .and_then(|position| error.get(position..index))
                .is_some_and(|value| {
                    value
                        .chars()
                        .any(|character| character.is_ascii_alphanumeric() || character == '_')
                });
            let after_index = index.saturating_add(3);
            let after_is_word = error
                .get(after_index..)
                .and_then(|value| value.chars().next())
                .is_some_and(|value| value.is_ascii_alphanumeric() || value == '_');
            !before_is_word && !after_is_word
        });
        let status_allows_number = status_code.is_none_or(|status| status == 429);
        let lower = error.to_ascii_lowercase();
        (standalone && status_allows_number)
            || lower.contains("rate limit")
            || lower.contains("rate-limit")
            || lower.contains("rate_limit")
            || lower.contains("service tier capacity exceeded")
    }

    #[must_use]
    pub fn context_window(error: &str) -> bool {
        let lower = error.to_ascii_lowercase();
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
        let lower = error.to_ascii_lowercase();
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
) -> Option<ProxyResponseFacts> {
    let status_code = error.status_code?;
    let response_headers = error.response_headers.clone().unwrap_or_default();
    if custom_provider != "litellm_proxy" || !response_headers.is_empty() {
        return Some(ProxyResponseFacts {
            status_code,
            headers: response_headers,
            request: error.request.clone(),
        });
    }
    let headers = error
        .headers
        .clone()
        .filter(|headers| !headers.is_empty())?;
    Some(ProxyResponseFacts {
        status_code,
        headers,
        request: error.request.clone(),
    })
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
