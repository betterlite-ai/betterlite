use std::collections::BTreeMap;
use std::fmt;

use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{Error as SerdeError, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use super::JsonValue;

macro_rules! define_json_deserializer {
    () => {
        struct JsonVisitor;

        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = JsonValue;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON value")
            }

            fn visit_unit<Error>(self) -> Result<Self::Value, Error>
            where
                Error: SerdeError,
            {
                Ok(JsonValue::Null)
            }

            fn visit_bool<Error>(self, value: bool) -> Result<Self::Value, Error>
            where
                Error: SerdeError,
            {
                Ok(JsonValue::Bool(value))
            }

            fn visit_i64<Error>(self, value: i64) -> Result<Self::Value, Error>
            where
                Error: SerdeError,
            {
                Ok(JsonValue::Number(serde_json::Number::from(value)))
            }

            fn visit_u64<Error>(self, value: u64) -> Result<Self::Value, Error>
            where
                Error: SerdeError,
            {
                Ok(JsonValue::Number(serde_json::Number::from(value)))
            }

            fn visit_f64<Error>(self, value: f64) -> Result<Self::Value, Error>
            where
                Error: SerdeError,
            {
                serde_json::Number::from_f64(value)
                    .map(JsonValue::Number)
                    .ok_or_else(|| Error::custom("non-finite JSON number"))
            }

            fn visit_str<Error>(self, value: &str) -> Result<Self::Value, Error>
            where
                Error: SerdeError,
            {
                Ok(JsonValue::String(value.to_owned()))
            }

            fn visit_string<Error>(self, value: String) -> Result<Self::Value, Error>
            where
                Error: SerdeError,
            {
                Ok(JsonValue::String(value))
            }

            fn visit_seq<Sequence>(self, sequence: Sequence) -> Result<Self::Value, Sequence::Error>
            where
                Sequence: SeqAccess<'de>,
            {
                Vec::deserialize(SeqAccessDeserializer::new(sequence)).map(JsonValue::Array)
            }

            fn visit_map<Map>(self, mut map: Map) -> Result<Self::Value, Map::Error>
            where
                Map: MapAccess<'de>,
            {
                let Some(key) = map.next_key::<String>()? else {
                    return Ok(JsonValue::Object(BTreeMap::new()));
                };
                // serde_json sends arbitrary-precision numbers through its private map representation.
                if key == "$serde_json::private::Number" {
                    let number = map.next_value::<String>()?;
                    return number
                        .parse()
                        .map(JsonValue::Number)
                        .map_err(Map::Error::custom);
                }
                let value = map.next_value()?;
                let mut values = BTreeMap::from([(key, value)]);
                values.extend(BTreeMap::<String, JsonValue>::deserialize(
                    MapAccessDeserializer::new(map),
                )?);
                Ok(JsonValue::Object(values))
            }
        }

        impl<'de> Deserialize<'de> for JsonValue {
            fn deserialize<Decoder>(deserializer: Decoder) -> Result<Self, Decoder::Error>
            where
                Decoder: Deserializer<'de>,
            {
                deserializer.deserialize_any(JsonVisitor)
            }
        }
    };
}

define_json_deserializer!();
