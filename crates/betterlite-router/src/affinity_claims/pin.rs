use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use betterlite_models::decisions::Json;

// the same 1 MiB bound the route value parser uses, checked before any decoding allocates
pub const MAX_PIN_BYTES: usize = 1024 * 1024;

// the pin a caller writes: one string value per field, keyed like LiteLLM's Mapping[str, str]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AffinityPin(pub IndexMap<String, String>);

impl AffinityPin {
    // the text LiteLLM's json.dumps(dict(pin)) gives its Redis claim script
    #[must_use]
    pub fn wire_text(&self) -> String {
        match serde_json::to_string(&self.0) {
            Ok(text) => text,
            Err(_error) => String::new(),
        }
    }
}

impl TryFrom<&Json> for AffinityPin {
    type Error = PinError;

    fn try_from(value: &Json) -> Result<Self, Self::Error> {
        let Json::Object(fields) = value else {
            return Err(PinError::NotFields);
        };
        let mut pin = IndexMap::with_capacity(fields.len());
        for (field, value) in fields {
            let Json::Text(text) = value else {
                return Err(PinError::NotFields);
            };
            pin.insert(field.clone(), text.clone());
        }
        Ok(Self(pin))
    }
}

// what a cache holds: a pin, a bare string, or any other JSON value older writers may have left
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoredPin {
    Pin(AffinityPin),
    Text(String),
    Other(Json),
}

impl StoredPin {
    #[must_use]
    pub fn from_json(value: Json) -> Self {
        match value {
            Json::Object(map) => {
                if map.values().all(|value| matches!(value, Json::Text(_))) {
                    let mut fields = IndexMap::with_capacity(map.len());
                    for (field, value) in map {
                        if let Json::Text(text) = value {
                            fields.insert(field, text);
                        }
                    }
                    Self::Pin(AffinityPin(fields))
                } else {
                    Self::Other(Json::Object(map))
                }
            }
            Json::Text(text) => Self::Text(text),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => Self::Other(value),
        }
    }

    // InMemoryCache.get_cache's json.loads, with the byte bound checked before the parse
    pub fn parse(bytes: &[u8]) -> Result<Self, PinError> {
        if bytes.len() > MAX_PIN_BYTES {
            return Err(PinError::TooLarge {
                bytes: bytes.len(),
                limit: MAX_PIN_BYTES,
            });
        }
        if let Ok(value) = serde_json::from_slice::<Json>(bytes) {
            return Ok(Self::from_json(value));
        }
        match std::str::from_utf8(bytes) {
            Ok(text) => Ok(Self::Text(text.to_owned())),
            Err(_error) => Err(PinError::NotText),
        }
    }

    #[must_use]
    pub fn wire_text(&self) -> String {
        match self {
            Self::Pin(pin) => pin.wire_text(),
            Self::Text(text) => text.clone(),
            Self::Other(value) => match serde_json::to_string(value) {
                Ok(text) => text,
                Err(_error) => String::new(),
            },
        }
    }

    #[must_use]
    pub fn decoded(&self) -> Json {
        match self {
            Self::Pin(pin) => Json::Object(
                pin.0
                    .iter()
                    .map(|(field, value)| (field.clone(), Json::Text(value.clone())))
                    .collect(),
            ),
            Self::Text(text) => Json::Text(text.clone()),
            Self::Other(value) => value.clone(),
        }
    }

    // claim_affinity_pin_in_memory's legacy check: extra stored fields are allowed
    #[must_use]
    pub fn matches_legacy(&self, pin: &AffinityPin) -> bool {
        match self {
            Self::Pin(stored) => pin
                .0
                .iter()
                .all(|(field, value)| stored.get(field) == Some(value)),
            Self::Other(Json::Object(stored)) => pin.0.iter().all(|(field, value)| {
                matches!(stored.get(field), Some(Json::Text(text)) if text == value)
            }),
            Self::Other(Json::List(_) | Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_))
            | Self::Text(_) => false,
        }
    }

    // the Redis script's table comparison: the stored key set must equal the candidate's
    #[must_use]
    pub fn matches_exactly(&self, pin: &AffinityPin) -> bool {
        match self {
            Self::Pin(stored) => {
                stored.len() == pin.0.len()
                    && pin
                        .0
                        .iter()
                        .all(|(field, value)| stored.get(field) == Some(value))
            }
            Self::Other(Json::Object(stored)) => {
                stored.len() == pin.0.len()
                    && pin.0.iter().all(|(field, value)| {
                        matches!(stored.get(field), Some(Json::Text(text)) if text == value)
                    })
            }
            Self::Other(Json::List(items)) => items.is_empty() && pin.0.is_empty(),
            Self::Other(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_))
            | Self::Text(_) => false,
        }
    }

    // Python's `existing in eligible_values`: plain equality, so an empty list never equals an empty pin
    #[must_use]
    pub fn equals_pin(&self, pin: &AffinityPin) -> bool {
        match self {
            Self::Pin(_) | Self::Other(Json::Object(_)) => self.matches_exactly(pin),
            Self::Other(
                Json::List(_) | Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_),
            )
            | Self::Text(_) => false,
        }
    }

    // _pinned_model_id: a dict's model_id or a bare string, never a non-string field value
    #[must_use]
    pub fn pinned_model_id(&self) -> Option<String> {
        match self {
            Self::Pin(pin) => pin.0.get("model_id").cloned(),
            Self::Text(text) => Some(text.clone()),
            Self::Other(Json::Object(stored)) => match stored.get("model_id") {
                Some(Json::Text(text)) => Some(text.clone()),
                Some(
                    Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
                )
                | None => None,
            },
            Self::Other(
                Json::List(_) | Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_),
            ) => None,
        }
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PinError {
    #[error("an affinity pin of {bytes} bytes is past the {limit}-byte limit")]
    TooLarge { bytes: usize, limit: usize },
    #[error("the stored affinity value is not UTF-8 text")]
    NotText,
    #[error("an affinity pin must map field names to strings")]
    NotFields,
}
