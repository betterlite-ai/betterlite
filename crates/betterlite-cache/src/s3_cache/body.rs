use serde_json::{Map, Value};

use super::{entry::CachedEntry, literal};

pub fn decode_body(bytes: &[u8]) -> Result<CachedEntry, DecodeError> {
    let value = serde_json::from_slice(bytes)
        .or_else(|_| literal::parse(bytes))
        .map(CachedEntry)
        .map_err(|source| DecodeError::Literal {
            source: Box::new(source),
        })?;
    if value.0.is_object() {
        return Ok(value);
    }
    to_dictionary(&value.0)
}

fn to_dictionary(value: &Value) -> Result<CachedEntry, DecodeError> {
    let entries = value.as_array().ok_or(DecodeError::Dictionary)?;
    let mut dictionary = Map::new();
    for entry in entries {
        if let Some(pair) = entry.as_array() {
            let [key, payload] = pair.as_slice() else {
                return Err(DecodeError::Dictionary);
            };
            let name = key.as_str().ok_or(DecodeError::Dictionary)?;
            dictionary.insert(name.to_owned(), payload.clone());
        } else if let Some(text) = entry.as_str() {
            let mut chars = text.chars();
            let key = chars.next().ok_or(DecodeError::Dictionary)?;
            let payload = chars.next().ok_or(DecodeError::Dictionary)?;
            if chars.next().is_some() {
                return Err(DecodeError::Dictionary);
            }
            dictionary.insert(key.to_string(), Value::String(payload.to_string()));
        } else {
            return Err(DecodeError::Dictionary);
        }
    }
    Ok(CachedEntry(Value::Object(dictionary)))
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("invalid Python cache literal")]
    Literal {
        #[source]
        source: Box<literal::LiteralError>,
    },
    #[error("cache value cannot convert to a JSON dictionary")]
    Dictionary,
}
