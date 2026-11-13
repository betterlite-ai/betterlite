use indexmap::IndexMap;
use serde_json::value::RawValue;

pub type Fields = IndexMap<String, Box<RawValue>>;
