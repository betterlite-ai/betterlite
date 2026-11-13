use std::collections::BTreeMap;

use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::error::SamplingConvertError;

// free-form JSON the MCP wire declares as an object, kept as text so a schema or tool input is forwarded unchanged
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct JsonObject(Box<RawValue>);

impl PartialEq for JsonObject {
    fn eq(&self, other: &Self) -> bool {
        self.0.get() == other.0.get()
    }
}

impl Eq for JsonObject {}

impl AsRef<str> for JsonObject {
    fn as_ref(&self) -> &str {
        self.0.get()
    }
}

impl TryFrom<Box<RawValue>> for JsonObject {
    type Error = SamplingConvertError;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let object = serde_json::from_str::<BTreeMap<String, IgnoredAny>>(raw.get());
        if object.is_err() {
            return Err(SamplingConvertError::JsonObjectExpected);
        }
        Ok(Self(raw))
    }
}
