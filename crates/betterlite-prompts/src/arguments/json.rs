use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

pub type Object = IndexMap<String, Json>;

// IndexMap keeps insertion order without the workspace-wide preserve_order feature this crate skips.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Json {
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    List(Vec<Self>),
    Object(Object),
}

impl Json {
    #[must_use]
    pub const fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text.as_str()),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::List(_) | Self::Object(_) => None,
        }
    }

    #[must_use]
    pub const fn as_object(&self) -> Option<&Object> {
        match self {
            Self::Object(object) => Some(object),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Text(_) | Self::List(_) => None,
        }
    }

    #[must_use]
    pub fn is_truthy(&self) -> bool {
        match self {
            Self::Null | Self::Bool(false) => false,
            Self::Bool(true) => true,
            Self::Number(number) => number.as_f64().is_some_and(|parsed| parsed != 0.0_f64),
            Self::Text(text) => !text.is_empty(),
            Self::List(items) => !items.is_empty(),
            Self::Object(fields) => !fields.is_empty(),
        }
    }
}

mod tests;
