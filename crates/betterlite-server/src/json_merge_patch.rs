use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;

mod tests;

pub const MAX_MERGE_DEPTH: u32 = 64;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum MergePatchError {
    #[error("JSON merge patch nesting exceeds the maximum depth of {max}")]
    TooDeep { max: u32 },
}

pub fn apply_json_merge_patch(
    target: &JsonValue,
    patch: &JsonValue,
) -> Result<JsonValue, MergePatchError> {
    validate_patch_depth(patch, 0)?;
    Ok(merge_patch(target, patch))
}

// the depth check runs before any merge work, and only an object patch is counted at each level
fn validate_patch_depth(patch: &JsonValue, depth: u32) -> Result<(), MergePatchError> {
    let JsonValue::Object(entries) = patch else {
        return Ok(());
    };
    if depth >= MAX_MERGE_DEPTH {
        return Err(MergePatchError::TooDeep {
            max: MAX_MERGE_DEPTH,
        });
    }
    for value in entries.values() {
        validate_patch_depth(value, depth.saturating_add(1))?;
    }
    Ok(())
}

fn merge_patch(target: &JsonValue, patch: &JsonValue) -> JsonValue {
    let JsonValue::Object(entries) = patch else {
        return patch.clone();
    };
    let mut merged: BTreeMap<String, JsonValue> = BTreeMap::new();
    if let JsonValue::Object(base) = target {
        merged.extend(base.iter().map(|(key, value)| (key.clone(), value.clone())));
    }
    for (key, value) in entries {
        if value.is_null() {
            merged.remove(key);
        } else {
            let base_value = target.as_object().and_then(|base| base.get(key.as_str()));
            merged.insert(
                key.clone(),
                merge_patch(base_value.unwrap_or(&JsonValue::Null), value),
            );
        }
    }
    JsonValue::Object(merged.into_iter().collect())
}
