use std::str::from_utf8;

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
    #[must_use]
    pub fn decoded(&self) -> Json {
        let mut fields = IndexMap::with_capacity(self.0.len());
        for (field, value) in &self.0 {
            fields.insert(field.clone(), Json::Text(value.clone()));
        }
        Json::Object(fields)
    }
}

impl TryFrom<&Json> for AffinityPin {
    type Error = PinError;

    fn try_from(value: &Json) -> Result<Self, Self::Error> {
        let Json::Object(fields) = value else {
            return Err(PinError::NotFields);
        };
        let mut pin = IndexMap::with_capacity(fields.len());
        for (field, field_value) in fields {
            let Json::Text(text) = field_value else {
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
        match AffinityPin::try_from(&value) {
            Ok(pin) => Self::Pin(pin),
            Err(_error) => match value {
                Json::Text(text) => Self::Text(text),
                other @ Json::Null
                | other @ Json::Bool(_)
                | other @ Json::Number(_)
                | other @ Json::List(_)
                | other @ Json::Object(_) => Self::Other(other),
            },
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
        match from_utf8(bytes) {
            Ok(text) => Ok(Self::Text(text.to_owned())),
            Err(_error) => Err(PinError::NotText),
        }
    }

    #[must_use]
    pub fn wire_text(&self) -> String {
        match self {
            Self::Pin(pin) => serde_json::to_string(&pin.0).unwrap_or_default(),
            Self::Text(text) => text.clone(),
            Self::Other(value) => serde_json::to_string(value).unwrap_or_default(),
        }
    }

    #[must_use]
    pub fn decoded(&self) -> Json {
        match self {
            Self::Pin(pin) => pin.decoded(),
            Self::Text(text) => Json::Text(text.clone()),
            Self::Other(value) => value.clone(),
        }
    }

    // claim_affinity_pin_in_memory's legacy check: extra stored fields are allowed, and one pin value may be a bare string
    #[must_use]
    pub fn matches_legacy(&self, pin: &AffinityPin) -> bool {
        if let Self::Text(text) = self {
            return pin.0.len() == 1 && pin.0.values().any(|value| value == text);
        }
        let Some(fields) = self.fields() else {
            return false;
        };
        fields.contains(pin)
    }

    // the Redis script's table comparison: the stored key set must equal the candidate's
    #[must_use]
    pub fn matches_exactly(&self, pin: &AffinityPin) -> bool {
        let Some(fields) = self.fields() else {
            return matches!(self, Self::Other(Json::List(items)) if items.is_empty() && pin.0.is_empty());
        };
        fields.field_count() == pin.0.len() && fields.contains(pin)
    }

    // Python's `existing in eligible_values`: plain equality, so an empty list never equals an empty pin
    #[must_use]
    pub fn equals_pin(&self, pin: &AffinityPin) -> bool {
        if self.fields().is_none() {
            return false;
        }
        self.matches_exactly(pin)
    }

    const fn fields(&self) -> Option<Fields<'_>> {
        match self {
            Self::Pin(pin) => Some(Fields::TextMap(&pin.0)),
            Self::Other(Json::Object(fields)) => Some(Fields::JsonMap(fields)),
            Self::Other(
                Json::List(_) | Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_),
            )
            | Self::Text(_) => None,
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

enum Fields<'map> {
    TextMap(&'map IndexMap<String, String>),
    JsonMap(&'map IndexMap<String, Json>),
}

impl Fields<'_> {
    fn field_count(&self) -> usize {
        match self {
            Self::TextMap(fields) => fields.len(),
            Self::JsonMap(fields) => fields.len(),
        }
    }

    fn contains(&self, pin: &AffinityPin) -> bool {
        match self {
            Self::TextMap(stored) => pin
                .0
                .iter()
                .all(|(field, value)| stored.get(field) == Some(value)),
            Self::JsonMap(stored) => pin.0.iter().all(|(field, value)| {
                matches!(stored.get(field), Some(Json::Text(text)) if text == value)
            }),
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
