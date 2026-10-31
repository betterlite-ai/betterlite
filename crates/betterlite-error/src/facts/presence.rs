use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Field<T> {
    #[default]
    #[serde(skip)]
    Missing,
    Null,
    Value(T),
}

impl<T> Field<T> {
    #[must_use]
    pub const fn is_missing(&self) -> bool {
        match self {
            Self::Missing => true,
            Self::Null | Self::Value(_) => false,
        }
    }

    #[must_use]
    pub const fn as_ref(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Missing | Self::Null => None,
        }
    }
}
