use std::collections::BTreeMap;

use super::entry::CachedEntry;

pub fn decode_body(bytes: &[u8]) -> Result<CachedEntry, DecodeError> {
    let value: CachedEntry =
        serde_json::from_slice(bytes).map_err(|source| DecodeError::Json { source })?;
    if value.0.is_object() {
        return Ok(value);
    }
    let entries = value.0.as_array().ok_or(DecodeError::Dictionary)?;
    let mut dictionary = BTreeMap::new();
    for entry in entries {
        if let Some(pair) = entry.as_array() {
            let [key, payload] = pair.as_slice() else {
                return Err(DecodeError::Dictionary);
            };
            let name = key.as_str().ok_or(DecodeError::Dictionary)?;
            dictionary.insert(name.to_owned(), CachedEntry(payload.clone()));
        } else if let Some(text) = entry.as_str() {
            let mut chars = text.chars();
            let key = chars.next().ok_or(DecodeError::Dictionary)?;
            let payload = chars.next().ok_or(DecodeError::Dictionary)?;
            if chars.next().is_some() {
                return Err(DecodeError::Dictionary);
            }
            dictionary.insert(
                key.to_string(),
                CachedEntry(
                    serde_json::to_value(payload.to_string())
                        .map_err(|source| DecodeError::Json { source })?,
                ),
            );
        } else {
            return Err(DecodeError::Dictionary);
        }
    }
    let object = serde_json::to_value(dictionary).map_err(|source| DecodeError::Json { source })?;
    Ok(CachedEntry(object))
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("invalid cache JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("cache value cannot convert to a JSON dictionary")]
    Dictionary,
}
