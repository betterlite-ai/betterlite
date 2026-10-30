use indexmap::IndexMap;
use serde::Serialize;
use serde_json::value::RawValue;

pub type Fields = IndexMap<String, Box<RawValue>>;

pub(super) fn encode<Head>(head: &Head) -> Result<Fields, serde_json::Error>
where
    Head: Serialize,
{
    let json = serde_json::to_string(head)?;
    serde_json::from_str(&json)
}
