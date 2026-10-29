use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::error::ImageModelsError;

// Bedrock's request bodies accept arbitrary provider-native keys passed through unchanged
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Field {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    List(Vec<Self>),
    Mapping(Params),
}

pub type Params = IndexMap<String, Field>;

impl Field {
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        if let Self::Text(text) = self {
            Some(text)
        } else {
            None
        }
    }

    // Python truthiness: None/empty string/empty collection/zero/False are falsy
    #[must_use]
    pub fn truthy(&self) -> bool {
        match self {
            Self::Null | Self::Bool(_) => matches!(self, Self::Bool(true)),
            Self::Int(value) => *value != 0,
            Self::Float(value) => *value != 0.0,
            Self::Text(text) => !text.is_empty(),
            Self::List(items) => !items.is_empty(),
            Self::Mapping(fields) => !fields.is_empty(),
        }
    }

    // Python's str() for the values that can reach an f-string in this module
    #[must_use]
    pub fn python_str(&self) -> String {
        if let Some(text) = self.as_str() {
            return text.to_owned();
        }
        if matches!(self, Self::List(_) | Self::Mapping(_)) {
            return "object".to_owned();
        }
        if let Self::Bool(flag) = self {
            return if *flag { "True" } else { "False" }.to_owned();
        }
        if let Self::Int(value) = self {
            return value.to_string();
        }
        if let Self::Float(value) = self {
            return value.to_string();
        }
        "None".to_owned()
    }
}

impl From<&str> for Field {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for Field {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<i64> for Field {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<bool> for Field {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<Params> for Field {
    fn from(value: Params) -> Self {
        Self::Mapping(value)
    }
}

// Mirrors dict.pop(key, {}); a present, non-mapping value fails here rather than being coerced
pub(super) fn pop_mapping(
    params: &mut Params,
    key: &'static str,
) -> Result<Params, ImageModelsError> {
    match params.shift_remove(key) {
        None => Ok(Params::new()),
        Some(Field::Mapping(mapping)) => Ok(mapping),
        Some(_) => Err(ImageModelsError::NotAMapping(key)),
    }
}
