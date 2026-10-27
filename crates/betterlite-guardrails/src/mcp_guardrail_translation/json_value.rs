use std::collections::BTreeMap;
use std::fmt::{self, Formatter};

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Number;

const PRIVATE_NUMBER_KEY: &str = "$serde_json::private::Number";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum McpValue {
    Object(BTreeMap<String, Self>),
    Null,
    Boolean(bool),
    Number(Number),
    Text(String),
    Array(Vec<Self>),
}

impl<'de> Deserialize<'de> for McpValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(McpValueVisitor)
    }
}

struct McpValueVisitor;

impl<'de> Visitor<'de> for McpValueVisitor {
    type Value = McpValue;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
        Ok(McpValue::Boolean(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
        Ok(McpValue::Number(Number::from(v)))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
        Ok(McpValue::Number(Number::from(v)))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let number = Number::from_f64(v).ok_or_else(|| de::Error::custom("not a JSON number"))?;
        Ok(McpValue::Number(number))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
        Ok(McpValue::Text(v.to_owned()))
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
        Ok(McpValue::Text(v))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(McpValue::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(McpValue::Null)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element()? {
            values.push(value);
        }
        Ok(McpValue::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = BTreeMap::new();
        while let Some((key, value)) = map.next_entry()? {
            entries.insert(key, value);
        }
        if let Some(number) = private_number(entries.get(PRIVATE_NUMBER_KEY))
            && entries.len() == 1
        {
            return Ok(McpValue::Number(number));
        }
        Ok(McpValue::Object(entries))
    }
}

fn private_number(value: Option<&McpValue>) -> Option<Number> {
    match value {
        Some(McpValue::Text(text)) => text.parse().ok(),
        _ => None,
    }
}

mod tests;
