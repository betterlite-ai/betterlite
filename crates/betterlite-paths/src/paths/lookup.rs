use crate::error::PathError;
use crate::paths::{Document, PathLimits};

pub fn find(
    limits: &PathLimits,
    document: &Document,
    key_path: &str,
) -> Result<Option<Document>, PathError> {
    limits.check_length(key_path)?;
    if key_path.is_empty() {
        return Ok(None);
    }
    let unprefixed = key_path.strip_prefix("metadata.").unwrap_or(key_path);
    // LiteLLM swaps escaped dots for NUL before splitting, so a literal NUL also reads as a dot
    let escaped = unprefixed.replace("\\.", "\0");
    limits.check_depth(escaped.split('.').count())?;
    let mut current = &document.json;
    for part in escaped.split('.') {
        let Some(child) = current.get(part.replace('\0', ".")) else {
            return Ok(None);
        };
        current = child;
    }
    Ok(Some(Document {
        json: current.clone(),
    }))
}

pub fn find_or(
    limits: &PathLimits,
    document: &Document,
    key_path: &str,
    default: Document,
) -> Result<Document, PathError> {
    let Some(found) = find(limits, document, key_path)? else {
        return Ok(default);
    };
    let wanted = &default.json;
    let value = &found.json;
    // Python's isinstance(found, type(default)); bool is a subclass of int
    let same_type = if wanted.is_null() {
        true
    } else if wanted.is_boolean() {
        value.is_boolean()
    } else if wanted.is_f64() {
        value.is_f64()
    } else if wanted.is_number() {
        value.is_boolean() || value.is_i64() || value.is_u64()
    } else if wanted.is_string() {
        value.is_string()
    } else if wanted.is_array() {
        value.is_array()
    } else {
        value.is_object()
    };
    Ok(if same_type { found } else { default })
}

mod tests;
