use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr as _;

#[path = "coercion/tests.rs"]
mod tests;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OtelModelValue {
    Null(()),
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl OtelModelValue {
    #[must_use]
    pub const fn as_object(&self) -> Option<&BTreeMap<String, Self>> {
        if let Self::Object(map) = self {
            Some(map)
        } else {
            None
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Self]> {
        if let Self::Array(items) = self {
            Some(items)
        } else {
            None
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        if let Self::String(text) = self {
            Some(text)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(value) = self {
            Some(*value)
        } else {
            None
        }
    }

    #[must_use]
    pub fn as_decimal(&self) -> Option<Decimal> {
        if let Self::Number(number) = self {
            return Decimal::from_str(&number.to_string()).ok();
        }
        if let Self::String(text) = self {
            return Decimal::from_str(&numeric_text(text)?).ok();
        }
        None
    }
}

#[must_use]
pub fn as_int(value: &OtelModelValue) -> Option<i64> {
    if let OtelModelValue::String(text) = value {
        return numeric_text(text)?.parse::<i64>().ok();
    }
    value.as_decimal()?.trunc().to_i64()
}

fn numeric_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let bytes = trimmed.as_bytes();
    if trimmed.is_empty() {
        return None;
    }
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'_' {
            let previous = index
                .checked_sub(1)
                .and_then(|position| bytes.get(position))?;
            let next_position = index.checked_add(1)?;
            let next = bytes.get(next_position)?;
            if !previous.is_ascii_digit() || !next.is_ascii_digit() {
                return None;
            }
        }
    }
    Some(trimmed.replace('_', ""))
}

#[must_use]
pub fn as_str_tuple(value: &OtelModelValue) -> Option<Vec<&str>> {
    match value {
        OtelModelValue::String(text) => Some(vec![text]),
        OtelModelValue::Array(items) => items
            .iter()
            .map(|item| match item {
                OtelModelValue::String(text) => Some(text.as_str()),
                OtelModelValue::Null(())
                | OtelModelValue::Bool(_)
                | OtelModelValue::Number(_)
                | OtelModelValue::Array(_)
                | OtelModelValue::Object(_) => None,
            })
            .collect(),
        OtelModelValue::Null(())
        | OtelModelValue::Bool(_)
        | OtelModelValue::Number(_)
        | OtelModelValue::Object(_) => None,
    }
}

use rust_decimal::prelude::ToPrimitive as _;
