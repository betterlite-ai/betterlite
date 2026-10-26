use serde_json::Value;

use crate::chat::execution::ChatError;

const CONTEXT_WINDOW_PHRASES: [&str; 9] = [
    "exceed context limit",
    "this model's maximum context length is",
    "string too long. expected a string with maximum length",
    "model's maximum context limit",
    "is longer than the model's context length",
    "input tokens exceed the configured limit",
    "`inputs` tokens + `max_new_tokens` must be",
    "exceeds the available context size",
    "exceeds the maximum number of tokens allowed",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderFailure {
    pub status: u16,
    pub error_type: String,
    pub param: Option<String>,
}

#[derive(Clone, Copy)]
enum Fields {
    FromBody,
    Fixed(&'static str),
    Fallback,
}

impl ChatError {
    #[must_use]
    pub fn failure(&self) -> ProviderFailure {
        match self {
            Self::UnknownModel { .. } | Self::Request { .. } => {
                failure(400, Fields::Fallback, None)
            }
            Self::UnsupportedProvider { .. } | Self::Streaming => {
                failure(501, Fields::Fallback, None)
            }
            Self::MissingApiKey => failure(401, Fields::Fallback, None),
            Self::Timeout { .. } => failure(408, Fields::Fallback, None),
            Self::Provider { response } => classify(
                response.status,
                str::from_utf8(&response.body).unwrap_or_default(),
            ),
            Self::Http { .. } | Self::Encode { .. } | Self::Decode { .. } | Self::TooLarge => {
                failure(500, Fields::Fallback, None)
            }
        }
    }
}

// LiteLLM checks the provider's error text before its status, in this order, so every 429 is a throttling_error
fn classify(status: u16, body: &str) -> ProviderFailure {
    let lower = body.to_lowercase();
    let invalid_request = body.contains("invalid_request_error");
    let (code, fields) = if status == 429 || mentions_rate_limit(&lower) {
        (429, Fields::Fixed("throttling_error"))
    } else if context_window_exceeded(&lower) {
        return failure(status, Fields::Fallback, None);
    } else if invalid_request && body.contains("model_not_found") {
        return failure(404, Fields::Fallback, None);
    } else if body.contains("A timeout occurred") {
        return failure(408, Fields::Fallback, None);
    } else if content_policy_violation(body, &lower)
        || body.contains("invalid_encrypted_content")
        || body.contains("could not be verified")
        || (invalid_request && !body.contains("Incorrect API key provided"))
    {
        (status, Fields::FromBody)
    } else if body.contains("Web server is returning an unknown error")
        || body.contains("The server had an error processing your request.")
    {
        (500, Fields::Fixed("internal_server_error"))
    } else if body.contains("Request too large") {
        (429, Fields::Fixed("throttling_error"))
    } else {
        return by_status(status, body);
    };
    failure(code, fields, Some(body))
}

fn by_status(status: u16, body: &str) -> ProviderFailure {
    match status {
        400 | 422 => failure(status, Fields::FromBody, Some(body)),
        500 => failure(500, Fields::Fixed("internal_server_error"), Some(body)),
        _ => failure(status, Fields::Fallback, None),
    }
}

fn failure(status: u16, fields: Fields, body: Option<&str>) -> ProviderFailure {
    let parsed = body.and_then(|text| serde_json::from_str::<Value>(text).ok());
    // the OpenAI SDK reads type and param from the error object, or from the whole body without one
    let error = parsed
        .as_ref()
        .map(|data| data.get("error").unwrap_or(data));
    let text_field = |name: &str| {
        error
            .and_then(|object| object.get(name))
            .and_then(Value::as_str)
            .filter(|text| *text != "None")
            .map(str::to_owned)
    };
    let error_type = match fields {
        Fields::Fixed(name) => Some(name.to_owned()),
        Fields::FromBody => text_field("type"),
        Fields::Fallback => None,
    };
    ProviderFailure {
        status,
        error_type: error_type.unwrap_or_else(|| fallback_type(status).to_owned()),
        param: text_field("param"),
    }
}

const fn fallback_type(status: u16) -> &'static str {
    match status {
        401 => "authentication_error",
        403 => "permission_error",
        0..500 => "invalid_request_error",
        _ => "internal_server_error",
    }
}

fn mentions_rate_limit(lower: &str) -> bool {
    lower.contains("service tier capacity exceeded")
        || lower.match_indices("rate").any(|(start, word)| {
            let rest = lower
                .get(start.saturating_add(word.len())..)
                .unwrap_or_default();
            rest.trim_start_matches(|letter: char| {
                letter.is_whitespace() || letter == '_' || letter == '-'
            })
            .starts_with("limit")
        })
}

fn context_window_exceeded(lower: &str) -> bool {
    if lower.contains("string_above_max_length")
        || (lower.contains("invalid 'user'") && lower.contains("string too long"))
    {
        return false;
    }
    CONTEXT_WINDOW_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
        || (lower.contains("current length is") && lower.contains("while limit is"))
        || (lower.contains("maximum input length is") && lower.contains("tokens"))
}

fn content_policy_violation(body: &str, lower: &str) -> bool {
    (body.contains("invalid_request_error") && body.contains("content_policy_violation"))
        || (body.contains("Invalid prompt") && body.contains("violating our usage policy"))
        || lower.contains("request was rejected as a result of the safety system")
}

mod tests;
