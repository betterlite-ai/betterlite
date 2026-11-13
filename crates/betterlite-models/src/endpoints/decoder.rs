use indexmap::IndexMap;
use serde_json::value::RawValue;

use super::{MAX_REGISTRY_DEPTH, RegistryError};

pub(super) fn validate(bytes: &[u8]) -> Result<(), RegistryError> {
    let entries: IndexMap<String, Box<RawValue>> = serde_json::from_slice(bytes)?;
    for value in entries.values() {
        validate_value(value, 1)?;
    }
    Ok(())
}

// RawValue bypasses serde_json's recursion limit, so bound recursive inspection explicitly.
fn validate_value(value: &RawValue, depth: usize) -> Result<(), RegistryError> {
    match value.get().as_bytes().first() {
        Some(b'{' | b'[') if depth >= MAX_REGISTRY_DEPTH => Err(RegistryError::TooDeep {
            limit: MAX_REGISTRY_DEPTH,
        }),
        Some(b'{') => {
            let fields: IndexMap<String, Box<RawValue>> = serde_json::from_str(value.get())?;
            for child in fields.values() {
                validate_value(child, depth.saturating_add(1))?;
            }
            Ok(())
        }
        Some(b'[') => {
            let elements: Vec<Box<RawValue>> = serde_json::from_str(value.get())?;
            for child in elements {
                validate_value(&child, depth.saturating_add(1))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
