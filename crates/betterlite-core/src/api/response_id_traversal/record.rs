use std::collections::BTreeSet;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

// A parsed output tree: a JSON value, or a typed object whose fields are attributes rather than dict keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Record {
    Object {
        #[serde(rename = "__object__")]
        object: AttributeObject,
    },
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    List(Vec<Self>),
    Dict(IndexMap<String, Self>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeObject {
    #[serde(default)]
    pub fields: IndexMap<String, Record>,
    // an attribute a Python object refuses to set raises AttributeError, which the caller may swallow
    #[serde(default)]
    pub readonly: BTreeSet<String>,
}

impl AttributeObject {
    pub fn set_text(&mut self, name: &str, value: String) -> Result<(), TraversalError> {
        if self.readonly.contains(name) {
            return Err(TraversalError::ReadOnly {
                field: name.to_owned(),
            });
        }
        self.fields.insert(name.to_owned(), Record::Text(value));
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TraversalError {
    #[error("the {field} attribute is read-only")]
    ReadOnly { field: String },
}

impl Record {
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&Self> {
        match self {
            Self::Dict(fields) => fields.get(name),
            Self::Object { object } => object.fields.get(name),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Text(_) | Self::List(_) => None,
        }
    }

    pub fn field_mut(&mut self, name: &str) -> Option<&mut Self> {
        match self {
            Self::Dict(fields) => fields.get_mut(name),
            Self::Object { object } => object.fields.get_mut(name),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Text(_) | Self::List(_) => None,
        }
    }

    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Null
            | Self::Bool(_)
            | Self::Number(_)
            | Self::List(_)
            | Self::Dict(_)
            | Self::Object { .. } => None,
        }
    }

    #[must_use]
    pub fn text_field(&self, name: &str) -> Option<&str> {
        match self.field(name) {
            Some(Self::Text(text)) => Some(text),
            Some(
                Self::Null
                | Self::Bool(_)
                | Self::Number(_)
                | Self::List(_)
                | Self::Dict(_)
                | Self::Object { .. },
            )
            | None => None,
        }
    }

    #[must_use]
    pub fn list_field(&self, name: &str) -> Option<&[Self]> {
        match self.field(name) {
            Some(Self::List(items)) => Some(items),
            Some(
                Self::Null
                | Self::Bool(_)
                | Self::Number(_)
                | Self::Text(_)
                | Self::Dict(_)
                | Self::Object { .. },
            )
            | None => None,
        }
    }

    pub fn list_field_mut(&mut self, name: &str) -> Option<&mut Vec<Self>> {
        match self.field_mut(name) {
            Some(Self::List(items)) => Some(items),
            Some(
                Self::Null
                | Self::Bool(_)
                | Self::Number(_)
                | Self::Text(_)
                | Self::Dict(_)
                | Self::Object { .. },
            )
            | None => None,
        }
    }

    pub fn set_text(&mut self, name: &str, value: String) -> Result<(), TraversalError> {
        match self {
            Self::Dict(fields) => {
                fields.insert(name.to_owned(), Self::Text(value));
                Ok(())
            }
            Self::Object { object } => object.set_text(name, value),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Text(_) | Self::List(_) => {
                Err(TraversalError::ReadOnly {
                    field: name.to_owned(),
                })
            }
        }
    }

    // Python truthiness for the summary block text check; a typed object is always true
    #[must_use]
    pub fn has_value(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(flag) => *flag,
            Self::Number(number) => number.as_f64().is_none_or(|value| value.abs() > 0.0_f64),
            Self::Text(text) => !text.is_empty(),
            Self::List(items) => !items.is_empty(),
            Self::Dict(fields) => !fields.is_empty(),
            Self::Object { .. } => true,
        }
    }
}
