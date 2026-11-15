use betterlite_prompts::arguments::json::Json;
use regex::Regex;

use crate::databricks_auth::Headers;

const SENSITIVE_PATTERNS: [(&str, &str); 6] = [
    (r"(?i)(Bearer\s+)[A-Za-z0-9_.-]+", "${1}[REDACTED]"),
    (r"(?i)(Authorization:\s*)[^\s,}]+", "${1}[REDACTED]"),
    (r#"(?i)(api[_-]?key["\s:=]+)[^\s,}"']+"#, "${1}[REDACTED]"),
    (
        r#"(?i)(client[_-]?secret["\s:=]+)[^\s,}"']+"#,
        "${1}[REDACTED]",
    ),
    ("(?i)(dapi[a-zA-Z0-9]{32,})", "[REDACTED_PAT]"),
    (
        r#"(?i)(access[_-]?token["\s:=]+)[^\s,}"']+"#,
        "${1}[REDACTED]",
    ),
];

const SENSITIVE_HEADERS: [&str; 4] = [
    "authorization",
    "x-api-key",
    "api-key",
    "x-databricks-token",
];

const SENSITIVE_KEY_PARTS: [&str; 7] = [
    "authorization",
    "api_key",
    "apikey",
    "token",
    "secret",
    "password",
    "credential",
];

fn redact_text(text: &str) -> String {
    let mut result = text.to_owned();
    for (pattern, replacement) in SENSITIVE_PATTERNS {
        let Ok(compiled) = Regex::new(pattern) else {
            // a constant pattern cannot fail to compile, so this arm never runs
            continue;
        };
        result = compiled.replace_all(&result, replacement).into_owned();
    }
    result
}

#[must_use]
pub fn redact_sensitive_data(data: &Json) -> Json {
    match data {
        Json::Null | Json::Bool(_) | Json::Number(_) => data.clone(),
        Json::Text(text) => Json::Text(redact_text(text)),
        Json::List(items) => Json::List(items.iter().map(redact_sensitive_data).collect()),
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, field)| {
                    if is_sensitive_key(name) {
                        (name.clone(), Json::Text("[REDACTED]".to_owned()))
                    } else {
                        (name.clone(), redact_sensitive_data(field))
                    }
                })
                .collect(),
        ),
    }
}

fn is_sensitive_key(name: &str) -> bool {
    // the parts match substrings, so a key like "max_tokens" is redacted along with "access_token"
    SENSITIVE_KEY_PARTS
        .iter()
        .any(|part| name.to_lowercase().contains(part))
}

#[must_use]
pub fn redact_headers_for_logging(headers: &Headers) -> Headers {
    headers
        .iter()
        .map(|(name, value)| {
            let sensitive = SENSITIVE_HEADERS
                .iter()
                .any(|candidate| name.eq_ignore_ascii_case(candidate));
            if !sensitive {
                return (name.clone(), value.clone());
            }
            let redacted = if value.chars().count() > 10 {
                let head: String = value.chars().take(8).collect();
                format!("{head}...[REDACTED]")
            } else {
                "[REDACTED]".to_owned()
            };
            (name.clone(), redacted)
        })
        .collect()
}
