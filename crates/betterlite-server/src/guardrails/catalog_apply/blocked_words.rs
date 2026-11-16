use std::collections::BTreeMap;

use axum::Json;
use axum::body::Bytes;
use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::router::unauthenticated;

use super::body::{BodyError, decode_body};

pub(super) const VALIDATE_PATH: &str = "/guardrails/validate_blocked_words_file";
const NO_CONTENT: &str = "No file content provided";
const NOT_A_FILE: &str = "Invalid format: file must contain 'blocked_words' key with a list";
const NOT_A_LIST: &str = "'blocked_words' must be a list";

#[derive(Debug, PartialEq, Eq)]
pub enum BlockedWordsVerdict {
    NoContent,
    Syntax { message: String },
    NotAFile,
    NotAList,
    Invalid { errors: Vec<String> },
    Valid { count: usize },
}

pub type ValidationOutcome = BlockedWordsVerdict;

#[derive(Serialize)]
struct VerdictBody {
    valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    errors: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

pub(super) async fn validate_blocked_words_file(
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    match validate_blocked_words(&body) {
        Ok(verdict) => (StatusCode::OK, Json(verdict_body(verdict))).into_response(),
        Err(error) => error.into_response(),
    }
}

pub fn validate_blocked_words(body: &[u8]) -> Result<ValidationOutcome, BodyError> {
    let fields: BTreeMap<String, String> = decode_body(body)?;
    let file_content = fields.get("file_content").map_or("", String::as_str);
    Ok(check_blocked_words(file_content))
}

#[must_use]
pub fn check_blocked_words(file_content: &str) -> BlockedWordsVerdict {
    if file_content.is_empty() {
        return BlockedWordsVerdict::NoContent;
    }
    let document = match serde_saphyr::from_str::<GuardrailValue>(file_content) {
        Ok(document) => document,
        Err(error) => {
            return BlockedWordsVerdict::Syntax {
                message: format!("Invalid YAML syntax: {error}"),
            };
        }
    };
    let GuardrailValue::Object(mut document) = document else {
        return BlockedWordsVerdict::NotAFile;
    };
    match document.remove("blocked_words") {
        None => BlockedWordsVerdict::NotAFile,
        Some(GuardrailValue::Array(entries)) => validate_entries(&entries),
        Some(_not_a_list) => BlockedWordsVerdict::NotAList,
    }
}

fn validate_entries(entries: &[GuardrailValue]) -> BlockedWordsVerdict {
    let mut errors = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        check_entry(index, entry, &mut errors);
    }
    if errors.is_empty() {
        BlockedWordsVerdict::Valid {
            count: entries.len(),
        }
    } else {
        BlockedWordsVerdict::Invalid { errors }
    }
}

fn check_entry(index: usize, entry: &GuardrailValue, errors: &mut Vec<String>) {
    let GuardrailValue::Object(fields) = entry else {
        errors.push(format!("Entry {index}: must be an object"));
        return;
    };
    match fields.get("keyword") {
        None => errors.push(format!("Entry {index}: missing 'keyword' field")),
        Some(GuardrailValue::Text(_keyword)) => {}
        Some(_not_text) => errors.push(format!("Entry {index}: 'keyword' must be a string")),
    }
    match fields.get("action") {
        None => errors.push(format!("Entry {index}: missing 'action' field")),
        Some(GuardrailValue::Text(action)) if action == "BLOCK" || action == "MASK" => {}
        Some(other) => errors.push(format!(
            "Entry {index}: action must be 'BLOCK' or 'MASK', got '{}'",
            value_text(other)
        )),
    }
    match fields.get("description") {
        None | Some(GuardrailValue::Text(_description)) => {}
        Some(_not_text) => errors.push(format!("Entry {index}: 'description' must be a string")),
    }
}

// Python prints the action value with str(); the scalars below keep that text, containers do not
fn value_text(value: &GuardrailValue) -> String {
    match value {
        GuardrailValue::Null => "None".to_owned(),
        GuardrailValue::Boolean(true) => "True".to_owned(),
        GuardrailValue::Boolean(false) => "False".to_owned(),
        GuardrailValue::SignedInteger(number) => number.to_string(),
        GuardrailValue::UnsignedInteger(number) => number.to_string(),
        // f64's Debug keeps the trailing .0 that Python's str prints for a whole float
        GuardrailValue::Decimal(number) => format!("{number:?}"),
        GuardrailValue::Text(text) => text.clone(),
        GuardrailValue::Array(items) => format!(
            "[{}]",
            items.iter().map(value_text).collect::<Vec<_>>().join(", ")
        ),
        GuardrailValue::Object(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(key, value)| format!("{key}: {}", value_text(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn verdict_body(verdict: BlockedWordsVerdict) -> VerdictBody {
    match verdict {
        BlockedWordsVerdict::NoContent => VerdictBody {
            valid: false,
            error: Some(NO_CONTENT.to_owned()),
            errors: None,
            message: None,
        },
        BlockedWordsVerdict::Syntax { message } => VerdictBody {
            valid: false,
            error: Some(message),
            errors: None,
            message: None,
        },
        BlockedWordsVerdict::NotAFile => VerdictBody {
            valid: false,
            error: Some(NOT_A_FILE.to_owned()),
            errors: None,
            message: None,
        },
        BlockedWordsVerdict::NotAList => VerdictBody {
            valid: false,
            error: Some(NOT_A_LIST.to_owned()),
            errors: None,
            message: None,
        },
        BlockedWordsVerdict::Invalid { errors } => VerdictBody {
            valid: false,
            error: None,
            errors: Some(errors),
            message: None,
        },
        BlockedWordsVerdict::Valid { count } => VerdictBody {
            valid: true,
            error: None,
            errors: None,
            message: Some(format!("Valid YAML file with {count} blocked word(s)")),
        },
    }
}
