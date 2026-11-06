use betterlite_redact::secret::Redactor;
use serde::Serialize;
use serde_json::Value;

use super::error::PrivateJsonError;

pub const DEFAULT_MAX_RECURSE_DEPTH: usize = 100;
pub const MAX_JSON_BYTES: usize = 0x0010_0000;
pub const MAX_JSON_DEPTH: usize = 64;
pub const MAX_DEPTH_MARKER: &str = "MaxDepthExceeded";

pub trait ValueTransform {
    fn transform(&self, key: Option<&str>, value: &str) -> String;
}

// B187's structured redaction is exactly LiteLLM's value_transform shape: the enclosing key comes with the value
impl ValueTransform for Redactor {
    fn transform(&self, key: Option<&str>, value: &str) -> String {
        self.redact_structured_value(key, value)
    }
}

// The named passthrough field: untyped integration JSON travels inside this type and never as a bare value in a record
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Passthrough {
    pub value: Value,
}

pub fn safe_dumps<T>(
    data: &T,
    max_depth: usize,
    transform: Option<&dyn ValueTransform>,
) -> Result<String, PrivateJsonError>
where
    T: Serialize,
{
    let value = serde_json::to_value(data).map_err(PrivateJsonError::Encode)?;
    let structure = Structure {
        max_depth,
        transform,
    }
    .value(&value, 0, None);
    serde_json::to_string(&structure).map_err(PrivateJsonError::Encode)
}

pub fn safe_json_loads(bytes: &[u8]) -> Result<Value, PrivateJsonError> {
    if bytes.len() > MAX_JSON_BYTES {
        return Err(PrivateJsonError::TooLarge(bytes.len(), MAX_JSON_BYTES));
    }
    let value: Value = serde_json::from_slice(bytes).map_err(PrivateJsonError::Decode)?;
    if container_depth(&value) > MAX_JSON_DEPTH {
        return Err(PrivateJsonError::TooDeep(MAX_JSON_DEPTH));
    }
    Ok(value)
}

struct Structure<'depth> {
    max_depth: usize,
    transform: Option<&'depth dyn ValueTransform>,
}

impl Structure<'_> {
    // safe_json_dumps.py's _serialize: depth first, so even a scalar past the bound becomes the marker
    fn value(&self, value: &Value, depth: usize, key: Option<&str>) -> Value {
        if depth > self.max_depth {
            return Value::String(MAX_DEPTH_MARKER.to_owned());
        }
        match value {
            Value::String(text) => Value::String(self.text(key, text)),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|item| self.value(item, depth.saturating_add(1), key))
                    .collect(),
            ),
            Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .map(|(name, item)| {
                        let clean_name = strip_nul(name);
                        let child = self.value(item, depth.saturating_add(1), Some(&clean_name));
                        (clean_name, child)
                    })
                    .collect(),
            ),
            Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        }
    }

    fn text(&self, key: Option<&str>, text: &str) -> String {
        let cleaned = strip_nul(text);
        let Some(transform) = self.transform else {
            return cleaned;
        };
        transform.transform(key, &cleaned)
    }
}

// PostgreSQL text and jsonb columns reject NUL bytes, so safe_json_dumps.py removes them before any writer sees them
fn strip_nul(text: &str) -> String {
    if text.contains('\0') {
        text.replace('\0', "")
    } else {
        text.to_owned()
    }
}

// Counts container nesting only, so a scalar at the innermost level does not push a document over the bound
fn container_depth(value: &Value) -> usize {
    match value {
        Value::Array(items) => deepest(items.iter()),
        Value::Object(fields) => deepest(fields.values()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => 0,
    }
}

fn deepest<'value, I>(children: I) -> usize
where
    I: Iterator<Item = &'value Value>,
{
    children
        .map(|child| container_depth(child).saturating_add(1))
        .max()
        .unwrap_or(1)
}
