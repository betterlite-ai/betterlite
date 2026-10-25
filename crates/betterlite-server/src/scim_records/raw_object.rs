use serde::de::{Error, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt;

// directory values stay raw, while repeated object keys must never overwrite earlier values
#[derive(Debug, Clone, Default, Serialize)]
#[serde(transparent)]
pub struct RawObject(pub BTreeMap<String, Box<RawValue>>);

struct ObjectVisitor;
impl<'de> Visitor<'de> for ObjectVisitor {
    type Value = RawObject;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object with unique keys")
    }
    fn visit_map<Access: MapAccess<'de>>(
        self,
        mut access: Access,
    ) -> Result<RawObject, Access::Error> {
        let mut fields = BTreeMap::new();
        while let Some(key) = access.next_key::<String>()? {
            match fields.entry(key) {
                Entry::Vacant(entry) => {
                    entry.insert(access.next_value()?);
                }
                Entry::Occupied(entry) => {
                    return Err(Access::Error::custom(format!(
                        "duplicate key: {}",
                        entry.key()
                    )));
                }
            }
        }
        Ok(RawObject(fields))
    }
}
impl<'de> Deserialize<'de> for RawObject {
    fn deserialize<Decoder: Deserializer<'de>>(decoder: Decoder) -> Result<Self, Decoder::Error> {
        decoder.deserialize_map(ObjectVisitor)
    }
}
