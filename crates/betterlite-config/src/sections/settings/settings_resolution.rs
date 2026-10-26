use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::descriptors::FieldSource;

#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Value(JsonValue),
    Absent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub value: SettingValue,
    pub source: FieldSource,
}

#[cfg(test)]
mod tests;

#[must_use]
pub fn resolve(yaml_value: Option<&JsonValue>, db_value: Option<&JsonValue>) -> Resolved {
    if let Some(value) = yaml_value {
        return Resolved { value: SettingValue::Value(value.clone()), source: FieldSource::Config };
    }
    db_value.filter(|value| !value.is_null()).map_or(
        Resolved { value: SettingValue::Absent, source: FieldSource::Unset },
        |value| Resolved { value: SettingValue::Value(value.clone()), source: FieldSource::Db },
    )
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonValue {
    Null,
    Boolean(bool),
    SignedInteger(i64),
    UnsignedInteger(u64),
    Decimal(f64),
    Text(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl JsonValue {
    pub(crate) const fn is_null(&self) -> bool { matches!(self, Self::Null) }
    pub(crate) fn as_str(&self) -> Option<&str> { if let Self::Text(value) = self { Some(value) } else { None } }
}

impl fmt::Display for JsonValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("null"),
            Self::Boolean(value) => value.fmt(f),
            Self::SignedInteger(value) => value.fmt(f),
            Self::UnsignedInteger(value) => value.fmt(f),
            Self::Decimal(value) => value.fmt(f),
            Self::Text(value) => f.write_str(value),
            Self::Array(_) | Self::Object(_) => f.write_str("[complex value]"),
        }
    }
}
