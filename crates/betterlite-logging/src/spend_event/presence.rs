use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

// A key present with a null value differs from an absent key, and Option cannot tell them apart
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Presence {
    Present(Json),
    #[default]
    Missing,
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
