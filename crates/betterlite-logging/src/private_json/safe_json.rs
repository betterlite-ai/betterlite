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

// The named passthrough field: untyped integration JSON enters a record only through this type, which bounds it
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Passthrough {
    value: Value,
}

impl TryFrom<Value> for Passthrough {
    type Error = PrivateJsonError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if container_depth(&value) > MAX_JSON_DEPTH {
            return Err(PrivateJsonError::TooDeep(MAX_JSON_DEPTH));
        }
        let encoded = serde_json::to_vec(&value).map_err(PrivateJsonError::Encode)?;
        if encoded.len() > MAX_JSON_BYTES {
            return Err(PrivateJsonError::TooLarge(encoded.len(), MAX_JSON_BYTES));
        }
        Ok(Self { value })
    }
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
    // The check runs first, so a scalar past the bound becomes the marker too
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
            // PostgreSQL text and jsonb columns reject NUL bytes, so safe_json_dumps.py strips them from keys and values
            Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .map(|(name, item)| {
                        let clean_name = name.replace('\0', "");
                        let child = self.value(item, depth.saturating_add(1), Some(&clean_name));
                        (clean_name, child)
                    })
                    .collect(),
            ),
            Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        }
    }

    fn text(&self, key: Option<&str>, text: &str) -> String {
        let cleaned = text.replace('\0', "");
        let Some(transform) = self.transform else {
            return cleaned;
        };
        transform.transform(key, &cleaned)
    }
}

// Counts container nesting only, so a scalar at the innermost level does not push a document over the bound
fn container_depth(value: &Value) -> usize {
    let mut pending = vec![(value, 0_usize)];
    let mut deepest = 0;
    while let Some((node, depth)) = pending.pop() {
        let next = depth.saturating_add(1);
        match node {
            Value::Array(items) => {
                pending.extend(items.iter().map(|item| (item, next)));
                deepest = deepest.max(next);
            }
            Value::Object(fields) => {
                pending.extend(fields.values().map(|field| (field, next)));
                deepest = deepest.max(next);
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    deepest
}
