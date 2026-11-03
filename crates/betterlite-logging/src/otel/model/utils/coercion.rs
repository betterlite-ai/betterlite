use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr as _;

#[path = "coercion/tests.rs"]
mod tests;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum JsonValue {
    Null(()),
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl JsonValue {
    #[must_use]
    pub const fn as_object(&self) -> Option<&BTreeMap<String, Self>> {
        match self {
            Self::Object(map) => Some(map),
            Self::Null(()) | Self::Bool(_) | Self::Number(_) | Self::String(_) | Self::Array(_) => {
                None
            }
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(items) => Some(items),
            Self::Null(())
            | Self::Bool(_)
            | Self::Number(_)
            | Self::String(_)
            | Self::Object(_) => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            Self::Null(()) | Self::Bool(_) | Self::Number(_) | Self::Array(_) | Self::Object(_) => {
                None
            }
        }
    }

    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            Self::Null(())
            | Self::Number(_)
            | Self::String(_)
            | Self::Array(_)
            | Self::Object(_) => None,
        }
    }
}

#[must_use]
pub fn as_int(value: &JsonValue) -> Option<i64> {
    match value {
        JsonValue::Number(number) => {
            if let Some(integer) = number.as_i64() {
                Some(integer)
            } else {
                Decimal::from_str(&number.to_string())
                    .ok()?
                    .trunc()
                    .to_i64()
            }
        }
        JsonValue::String(text) => text.parse::<i64>().ok(),
        JsonValue::Null(()) | JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            None
        }
    }
}

#[must_use]
pub fn as_float(value: &JsonValue) -> Option<Decimal> {
    match value {
        JsonValue::Number(number) => Decimal::from_str(&number.to_string()).ok(),
        JsonValue::String(text) => Decimal::from_str(text).ok(),
        JsonValue::Null(()) | JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            None
        }
    }
}

#[must_use]
pub fn as_str_tuple(value: &JsonValue) -> Option<Vec<&str>> {
    match value {
        JsonValue::String(text) => Some(vec![text]),
        JsonValue::Array(items) => items
            .iter()
            .map(|item| match item {
                JsonValue::String(text) => Some(text.as_str()),
                JsonValue::Null(())
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::Array(_)
                | JsonValue::Object(_) => None,
            })
            .collect(),
        JsonValue::Null(()) | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Object(_) => {
            None
        }
    }
}

use rust_decimal::prelude::ToPrimitive as _;
