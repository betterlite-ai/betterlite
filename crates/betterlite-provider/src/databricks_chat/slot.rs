use serde::{Deserialize, Serialize};

// A JSON key as LiteLLM reads a dict: missing, null, or carrying a value
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Slot<Item> {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Null,
    Value(Item),
}

impl<Item> Slot<Item> {
    pub const fn is_absent(&self) -> bool {
        match self {
            Self::Absent => true,
            Self::Null | Self::Value(_) => false,
        }
    }
}
