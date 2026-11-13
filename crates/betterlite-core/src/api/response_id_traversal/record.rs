use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::api::responses_tool_namespaces::values::has_value as json_has_value;

// A parsed output tree: a JSON value, or a typed object whose fields are attributes rather than mapping keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Record {
    Object {
        #[serde(rename = "__object__")]
        object: AttributeObject,
    },
    Dict(DictRecord),
    List(Vec<Self>),
    Scalar(Json),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DictRecord {
    pub values: IndexMap<String, Record>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeObject {
    #[serde(default)]
    pub fields: IndexMap<String, Record>,
    // an attribute a Python object refuses to set raises AttributeError, which the caller may swallow
    #[serde(default)]
    pub readonly: BTreeSet<String>,
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
            Self::Dict(dict) => dict.values.get(name),
            Self::Object { object } => object.fields.get(name),
            Self::List(_) | Self::Scalar(_) => None,
        }
    }

    pub fn field_mut(&mut self, name: &str) -> Option<&mut Self> {
        match self {
            Self::Dict(dict) => dict.values.get_mut(name),
            Self::Object { object } => object.fields.get_mut(name),
            Self::List(_) | Self::Scalar(_) => None,
        }
    }

    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Scalar(Json::Text(text)) => Some(text),
            Self::Dict(_) | Self::Object { .. } | Self::List(_) | Self::Scalar(_) => None,
        }
    }

    pub fn set_text(&mut self, name: &str, value: String) -> Result<(), TraversalError> {
        let fields = match self {
            Self::Dict(dict) => &mut dict.values,
            Self::Object { object } => {
                if object.readonly.contains(name) {
                    return Err(TraversalError::ReadOnly {
                        field: name.to_owned(),
                    });
                }
                &mut object.fields
            }
            Self::List(_) | Self::Scalar(_) => {
                return Err(TraversalError::ReadOnly {
                    field: name.to_owned(),
                });
            }
        };
        fields.insert(name.to_owned(), Self::Scalar(Json::Text(value)));
        Ok(())
    }

    // Python truthiness for the summary block text check; a typed object is always true
    #[must_use]
    pub fn has_value(&self) -> bool {
        match self {
            Self::Dict(dict) => !dict.values.is_empty(),
            Self::Object { .. } => true,
            Self::List(items) => !items.is_empty(),
            Self::Scalar(value) => json_has_value(value),
        }
    }
}
