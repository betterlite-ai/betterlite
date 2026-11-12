use betterlite_models::decisions::Json;
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
        let found = match self {
            Self::Dict(map) => map.get(key),
            Self::Instance { object } => match &object.get {
                Getter::Mapping(map) => map.get(key),
                Getter::Raises(Raised::Other) => return Err(RepairError::GetterFailed),
                Getter::Absent | Getter::Raises(Raised::TypeError | Raised::AttributeError) => {
                    object.attrs.get(key)
                }
            },
            Self::Null
            | Self::Bool(_)
            | Self::Number(_)
            | Self::Text(_)
            | Self::Bytes { .. }
            | Self::List(_)
            | Self::Tuple { .. } => None,
        };
        Ok(found.filter(|value| **value != Self::Null))
    }

    // dict.get for a dict and getattr for an object
    #[must_use]
    pub fn plain_field(&self, key: &str) -> Option<&Self> {
        let found = match self {
            Self::Dict(map) => map.get(key),
            Self::Instance { object } => object.attrs.get(key),
            Self::Null
            | Self::Bool(_)
            | Self::Number(_)
            | Self::Text(_)
            | Self::Bytes { .. }
            | Self::List(_)
            | Self::Tuple { .. } => None,
        };
        found.filter(|value| **value != Self::Null)
    }

    // LiteLLM reads message.get("role"), so a message without a working get fails
    pub fn has_role(&self, role: &str) -> Result<bool, RepairError> {
        let found = match self {
            Self::Dict(map) => map.get("role"),
            Self::Instance { object } => match &object.get {
                Getter::Mapping(map) => map.get("role"),
                Getter::Absent => return Err(RepairError::NoGetMethod),
                Getter::Raises(_) => return Err(RepairError::GetterFailed),
            },
            Self::Null
            | Self::Bool(_)
            | Self::Number(_)
            | Self::Text(_)
            | Self::Bytes { .. }
            | Self::List(_)
            | Self::Tuple { .. } => return Err(RepairError::NoGetMethod),
        };
        Ok(matches!(found, Some(Self::Text(text)) if text == role))
    }

    // a dict gains the key; an object only changes an attribute it already has
    pub fn set_tool_call_id(&mut self, tool_call_id: &str) {
        match self {
            Self::Dict(map) => {
                map.insert(
                    "tool_call_id".to_owned(),
                    Self::Text(tool_call_id.to_owned()),
                );
            }
            Self::Instance { object } => {
                if let Some(slot) = object.attrs.get_mut("tool_call_id") {
                    *slot = Self::Text(tool_call_id.to_owned());
                }
            }
            Self::Null
            | Self::Bool(_)
            | Self::Number(_)
            | Self::Text(_)
            | Self::Bytes { .. }
            | Self::List(_)
            | Self::Tuple { .. } => {}
        }
    }

    // str(value) for the ids, types and names of a call
    pub fn text(&self, field: &'static str) -> Result<String, RepairError> {
        match self {
            Self::Text(text) => Ok(text.clone()),
            Self::Number(number) => Ok(number.to_string()),
            Self::Null
            | Self::Bool(_)
            | Self::Bytes { .. }
            | Self::List(_)
            | Self::Tuple { .. }
            | Self::Dict(_)
            | Self::Instance { .. } => Err(RepairError::WrongType { field }),
        }
    }

    pub fn to_json(&self, field: &'static str) -> Result<Json, RepairError> {
        match self {
            Self::Null => Ok(Json::Null),
            Self::Bool(flag) => Ok(Json::Bool(*flag)),
            Self::Number(number) => Ok(Json::Number(number.clone())),
            Self::Text(text) => Ok(Json::Text(text.clone())),
            Self::List(items) | Self::Tuple { items } => Ok(Json::List(
                items
                    .iter()
                    .map(|item| item.to_json(field))
                    .collect::<Result<_, _>>()?,
            )),
            Self::Dict(map) => Ok(Json::Object(
                map.iter()
                    .map(|(key, item)| Ok((key.clone(), item.to_json(field)?)))
                    .collect::<Result<_, _>>()?,
            )),
            Self::Bytes { .. } | Self::Instance { .. } => Err(RepairError::WrongType { field }),
        }
    }
}
