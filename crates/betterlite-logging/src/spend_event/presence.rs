use betterlite_models::decisions::Json;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

// A key present with a null value differs from an absent key, and Option cannot tell them apart
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Presence {
    #[default]
    Missing,
    Present(Json),
}

impl Presence {
    #[must_use]
    pub const fn is_missing(&self) -> bool {
        match self {
            Self::Missing => true,
            Self::Present(_) => false,
        }
    }
}

impl Serialize for Presence {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Missing => serializer.serialize_none(),
            Self::Present(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Presence {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Json::deserialize(deserializer).map(Self::Present)
    }
}
