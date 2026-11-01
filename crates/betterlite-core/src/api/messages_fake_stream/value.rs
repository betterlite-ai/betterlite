use std::ops::Index;

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Number;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Json {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Self>),
    Object(IndexMap<String, Self>),
}

impl Json {
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(fields) => fields.get(key),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::String(_) | Self::Array(_) => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Array(_) | Self::Object(_) => None,
        }
    }

    #[must_use]
    pub const fn as_object(&self) -> Option<&IndexMap<String, Self>> {
        match self {
            Self::Object(fields) => Some(fields),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::String(_) | Self::Array(_) => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(items) => Some(items),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::String(_) | Self::Object(_) => {
                None
            }
        }
    }

    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(number) => number.as_i64(),
            Self::Null | Self::Bool(_) | Self::String(_) | Self::Array(_) | Self::Object(_) => None,
        }
    }
}

impl Index<&str> for Json {
    type Output = Self;

    fn index(&self, index: &str) -> &Self {
        const NULL: Json = Json::Null;
        self.get(index).unwrap_or(&NULL)
    }
}

impl From<&str> for Json {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for Json {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<i64> for Json {
    fn from(value: i64) -> Self {
        Self::Number(Number::from(value))
    }
}

impl From<usize> for Json {
    fn from(value: usize) -> Self {
        Self::Number(Number::from(value))
    }
}
