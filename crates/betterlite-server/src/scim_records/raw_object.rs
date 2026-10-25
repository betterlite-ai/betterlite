use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_with::{Map, serde_as};
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use thiserror::Error;

// directory values stay raw, while repeated object keys must never overwrite earlier values
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(try_from = "ObjectEntries")]
pub struct RawObject(pub BTreeMap<String, Box<RawValue>>);

#[serde_as]
#[derive(Deserialize)]
struct ObjectEntries(#[serde_as(as = "Map<_, _>")] Vec<(String, Box<RawValue>)>);

#[derive(Debug, Error)]
pub enum ObjectError {
    #[error("duplicate key: {0}")]
    Duplicate(String),
}
impl TryFrom<ObjectEntries> for RawObject {
    type Error = ObjectError;
    fn try_from(entries: ObjectEntries) -> Result<Self, ObjectError> {
        let mut fields = BTreeMap::new();
        for (key, value) in entries.0 {
            match fields.entry(key) {
                Entry::Vacant(entry) => {
                    entry.insert(value);
                }
                Entry::Occupied(entry) => {
                    return Err(ObjectError::Duplicate(entry.key().clone()));
                }
            }
        }
        Ok(Self(fields))
    }
}
