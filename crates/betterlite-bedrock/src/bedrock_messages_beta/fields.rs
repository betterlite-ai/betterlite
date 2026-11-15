use betterlite_logging::analytics::python::{FormatError, repr};
use betterlite_prompts::arguments::json::Object;
use thiserror::Error;

pub const ALLOWED_TOP_LEVEL_FIELDS: [&str; 16] = [
    "anthropic_version",
    "max_tokens",
    "messages",
    "anthropic_beta",
    "system",
    "stop_sequences",
    "temperature",
    "top_p",
    "top_k",
    "tools",
    "tool_choice",
    "thinking",
    "metadata",
    "output_config",
    "safeguards",
    "context_management",
];

#[derive(Debug, Error)]
pub enum FieldsError {
    #[error("cannot render the stripped top-level field names")]
    Format(#[from] FormatError),
}

pub fn strip_unsupported_fields(request: &Object) -> Result<Object, FieldsError> {
    let mut kept = Object::new();
    let mut stripped = Vec::new();
    for (name, value) in request {
        if ALLOWED_TOP_LEVEL_FIELDS.contains(&name.as_str()) {
            kept.insert(name.clone(), value.clone());
        } else {
            stripped.push(name.clone());
        }
    }
    if stripped.is_empty() {
        return Ok(kept);
    }
    stripped.sort_unstable();
    let names = repr(&stripped)?;
    tracing::debug!(
        "Bedrock Invoke: stripping unsupported top-level request fields: {}",
        names
    );
    Ok(kept)
}
