use std::collections::BTreeSet;

use betterlite_bedrock::claude_platform::error::{ChatValidationError, NativeHeaderError};
use betterlite_bedrock::claude_platform::{AUTHENTICATION_PROVIDER, WORKSPACE_HEADER};
use betterlite_bedrock::claude_platform_inputs::body::PlatformWarning;
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

const BETA_HEADER: &str = "anthropic-beta";
const BILLING_MARKER: &str = "x-anthropic-billing-header";

pub(super) fn beta_tokens(value: &str) -> Vec<String> {
    let mut tokens = BTreeSet::new();
    for piece in value.split(',') {
        let token = piece.trim();
        if !token.is_empty() {
            tokens.insert(token.to_owned());
        }
    }
    tokens.into_iter().collect()
}

// Header names sort, and the beta value compares as the set of its comma-separated tokens.
pub(super) fn project_headers(headers: &IndexMap<String, String>) -> Value {
    let mut names: Vec<&String> = headers.keys().collect();
    names.sort();
    let mut projected = Vec::with_capacity(names.len());
    for name in names {
        let Some(value) = headers.get(name.as_str()) else {
            continue;
        };
        let rendered = if name.as_str() == BETA_HEADER {
            Value::Array(beta_tokens(value).into_iter().map(Value::String).collect())
        } else {
            Value::String(value.clone())
        };
        projected.push(json!([name, rendered]));
    }
    Value::Array(projected)
}

pub(super) fn warning_json(warning: &PlatformWarning) -> Value {
    match warning {
        PlatformWarning::DroppedUnsupported { params } => {
            json!({"action": "drop", "params": params})
        }
        PlatformWarning::IgnoredOverride { type_name } => {
            json!({"action": "ignore", "type_name": type_name})
        }
    }
}

fn beta_view(headers: &IndexMap<String, String>) -> Value {
    match headers.get(BETA_HEADER) {
        Some(value) => Value::Array(beta_tokens(value).into_iter().map(Value::String).collect()),
        None => Value::Array(Vec::new()),
    }
}

fn system_texts(body: Option<&Map<String, Value>>) -> Vec<String> {
    let mut texts = Vec::new();
    let Some(blocks) = body
        .and_then(|fields| fields.get("system"))
        .and_then(Value::as_array)
    else {
        return texts;
    };
    for block in blocks {
        if block.get("type").and_then(Value::as_str) != Some("text") {
            continue;
        }
        if let Some(text) = block.get("text").and_then(Value::as_str) {
            texts.push(text.to_owned());
        }
    }
    texts
}

fn removed(fields: Option<&Map<String, Value>>, key: &str) -> bool {
    fields.is_none_or(|entries| !entries.contains_key(key))
}

// The fields the cases pin: the model, the token budget, the filtered keys, the system text and the billing marker.
pub(super) fn body_view(
    body: &Value,
    warnings: &[PlatformWarning],
    headers: &IndexMap<String, String>,
) -> Value {
    let fields = body.as_object();
    let encoded = body.to_string();
    json!({
        "model": fields.and_then(|entries| entries.get("model")).cloned().unwrap_or(Value::Null),
        "max_tokens": fields.and_then(|entries| entries.get("max_tokens")).cloned().unwrap_or(Value::Null),
        "workspace_removed": removed(fields, "workspace_id"),
        "context_removed": removed(fields, "context_management"),
        "override_removed": removed(fields, "claude_platform_unsupported_params"),
        "system_texts": system_texts(fields),
        "billing": encoded.contains(BILLING_MARKER),
        "beta": beta_view(headers),
        "warnings": warnings.iter().map(warning_json).collect::<Vec<_>>(),
    })
}

pub(super) fn workspace_view(headers: &IndexMap<String, String>) -> Value {
    json!({"workspace": headers.get(WORKSPACE_HEADER)})
}

pub(super) fn chat_failure(error: &ChatValidationError) -> Value {
    let (model, message) = match error {
        ChatValidationError::Workspace(workspace) => {
            (workspace.model().map(str::to_owned), workspace.to_string())
        }
        ChatValidationError::Features(source) => (None, source.to_string()),
        ChatValidationError::Json(source) => (None, source.to_string()),
    };
    json!({
        "status": error.status_code(),
        "provider": AUTHENTICATION_PROVIDER,
        "model": model.unwrap_or_default(),
        "message": message,
    })
}

pub(super) fn native_header_failure(error: &NativeHeaderError) -> Value {
    let (model, message) = match error {
        NativeHeaderError::Workspace(workspace) => {
            (workspace.model().map(str::to_owned), workspace.to_string())
        }
        NativeHeaderError::Beta(source) => (None, source.to_string()),
        NativeHeaderError::Json(source) => (None, source.to_string()),
    };
    json!({
        "status": error.status_code(),
        "provider": AUTHENTICATION_PROVIDER,
        "model": model.unwrap_or_default(),
        "message": message,
    })
}
