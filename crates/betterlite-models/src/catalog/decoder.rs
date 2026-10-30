use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::value::RawValue;

use super::{CatalogError, MAX_CATALOG_DEPTH};

#[derive(Deserialize)]
struct UniqueMap(
    #[serde(
        deserialize_with = "serde_with::As::<serde_with::MapPreventDuplicates<serde_with::Same, serde_with::Same>>::deserialize"
    )]
    BTreeMap<String, Box<RawValue>>,
);

pub(super) fn decode(bytes: &[u8]) -> Result<IndexMap<String, Box<RawValue>>, CatalogError> {
    let UniqueMap(entries) = serde_json::from_slice(bytes)?;
    for value in entries.values() {
        validate(value, 1)?;
    }
    // Alias precedence follows declaration order after duplicate validation.
    Ok(serde_json::from_slice(bytes)?)
}

// RawValue bypasses serde_json's recursion limit, so bound recursive inspection explicitly.
fn validate(value: &RawValue, depth: usize) -> Result<(), CatalogError> {
    match value.get().as_bytes().first() {
        Some(b'{' | b'[') if depth >= MAX_CATALOG_DEPTH => Err(CatalogError::TooDeep {
            limit: MAX_CATALOG_DEPTH,
        }),
        Some(b'{') => {
            let UniqueMap(fields) = serde_json::from_str(value.get())?;
            for child in fields.values() {
                validate(child, depth.saturating_add(1))?;
            }
            Ok(())
        }
        Some(b'[') => {
            let elements: Vec<Box<RawValue>> = serde_json::from_str(value.get())?;
            for child in elements {
                validate(&child, depth.saturating_add(1))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
