use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::error::RepairError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Raised {
    TypeError,
    AttributeError,
    Other,
}

// What calling obj.get(key, default) does on an attribute object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Getter {
    #[default]
    Absent,
    Mapping(IndexMap<String, Value>),
    Raises(Raised),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    pub attrs: IndexMap<String, Value>,
    #[serde(default)]
    pub get: Getter,
}

// The Python values LiteLLM's helpers dispatch on: each message, call and definition may be a dict or an attribute object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Instance {
        #[serde(rename = "__object__")]
        object: Instance,
    },
    Tuple {
        #[serde(rename = "__tuple__")]
        items: Vec<Self>,
    },
    Bytes {
        #[serde(rename = "__bytes__")]
        bytes: String,
    },
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    List(Vec<Self>),
    Dict(IndexMap<String, Self>),
}

impl Value {
    #[must_use]
    pub fn has_content(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(flag) => *flag,
            Self::Number(number) => number.as_f64().is_none_or(|float| float.abs() > 0.0_f64),
            Self::Text(text) => !text.is_empty(),
            Self::Bytes { bytes } => !bytes.is_empty(),
            Self::List(items) | Self::Tuple { items } => !items.is_empty(),
            Self::Dict(map) => !map.is_empty(),
            Self::Instance { .. } => true,
        }
    }

    // dict.get, then a callable get(key, default), then the attribute when get raises TypeError or AttributeError
    pub fn field(&self, key: &str) -> Result<Option<&Self>, RepairError> {
        let Self::Instance { object } = self else {
            return Ok(self.plain_field(key));
        };
        match &object.get {
            Getter::Mapping(map) => Ok(map.get(key).filter(|found| **found != Self::Null)),
            Getter::Raises(Raised::Other) => Err(RepairError::GetterFailed),
            Getter::Absent | Getter::Raises(Raised::TypeError | Raised::AttributeError) => {
                Ok(self.plain_field(key))
            }
        }
    }

    // dict.get for a dict and getattr for an object
    #[must_use]
    pub fn plain_field(&self, key: &str) -> Option<&Self> {
        let entries = if let Self::Dict(map) = self {
            map
        } else if let Self::Instance { object } = self {
            &object.attrs
        } else {
            return None;
        };
        entries.get(key).filter(|found| **found != Self::Null)
    }

    // LiteLLM reads message.get("role"), so a message without a working get fails
    pub fn has_role(&self, role: &str) -> Result<bool, RepairError> {
        if let Self::Instance { object } = self {
            return match &object.get {
                Getter::Mapping(map) => {
                    Ok(matches!(map.get("role"), Some(Self::Text(text)) if text == role))
                }
                Getter::Absent => Err(RepairError::NoGetMethod),
                Getter::Raises(_) => Err(RepairError::GetterFailed),
            };
        }
        let Self::Dict(map) = self else {
            return Err(RepairError::NoGetMethod);
        };
        Ok(matches!(map.get("role"), Some(Self::Text(text)) if text == role))
    }

    // a dict gains the key; an object only changes an attribute it already has
    pub fn set_tool_call_id(&mut self, tool_call_id: &str) {
        if let Self::Dict(map) = self {
            map.insert(
                "tool_call_id".to_owned(),
                Self::Text(tool_call_id.to_owned()),
            );
        }
        if let Self::Instance { object } = self
            && let Some(slot) = object.attrs.get_mut("tool_call_id")
        {
            *slot = Self::Text(tool_call_id.to_owned());
        }
    }

    // str(value) for the ids, types and names of a call
    pub fn text(&self, field: &'static str) -> Result<String, RepairError> {
        if let Self::Text(text) = self {
            Ok(text.clone())
        } else if let Self::Number(number) = self {
            Ok(number.to_string())
        } else {
            Err(RepairError::WrongType { field })
        }
    }
}
