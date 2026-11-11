use crate::controls::json_value::JsonValue;

pub const MAX_EXTRA_BODY_BYTES: usize = 1024 * 1024;
pub const MAX_EXTRA_BODY_DEPTH: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum ExtraBodyError {
    #[error("extra_body is {size} bytes, over the {limit} byte limit")]
    TooLarge { size: usize, limit: usize },
    #[error("extra_body nesting exceeds {limit}")]
    TooDeep { limit: usize },
    #[error("extra_body could not be serialized: {source}")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
}

// LiteLLM forwards extra_body to the provider as it arrived; betterlite keeps the payload unchanged and only rejects it past the size and depth bounds.
pub fn check_extra_body(value: &JsonValue) -> Result<(), ExtraBodyError> {
    let size = serde_json::to_vec(value)
        .map_err(|source| ExtraBodyError::Serialize { source })?
        .len();
    if size > MAX_EXTRA_BODY_BYTES {
        return Err(ExtraBodyError::TooLarge {
            size,
            limit: MAX_EXTRA_BODY_BYTES,
        });
    }
    if depth_exceeds(value, MAX_EXTRA_BODY_DEPTH) {
        return Err(ExtraBodyError::TooDeep {
            limit: MAX_EXTRA_BODY_DEPTH,
        });
    }
    Ok(())
}

fn depth_exceeds(value: &JsonValue, limit: usize) -> bool {
    let mut pending: Vec<(&JsonValue, usize)> = vec![(value, 0)];
    while let Some((current, depth)) = pending.pop() {
        if depth > limit {
            return true;
        }
        match current {
            JsonValue::Array(items) => {
                pending.extend(items.iter().map(|item| (item, depth.saturating_add(1))));
            }
            JsonValue::Object(fields) => pending.extend(
                fields
                    .values()
                    .map(|field| (field, depth.saturating_add(1))),
            ),
            JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) => {}
        }
    }
    false
}

mod tests;
