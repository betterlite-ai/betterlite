use serde::{Deserialize, Serialize};
use thiserror::Error;

// A field whose Python type excludes None: absent from the input, or a value. The from type reads the value directly so its own error survives, and an explicit null fails there too.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(from = "T")]
pub struct Given<T>(Option<T>);

impl<T> Given<T> {
    pub(crate) const fn is_missing(&self) -> bool {
        match self {
            Self(None) => true,
            Self(Some(_)) => false,
        }
    }

    pub(crate) fn full(&self, default: T) -> Self
    where
        T: Clone,
    {
        match self {
            Self(None) => Self(Some(default)),
            Self(Some(value)) => Self(Some(value.clone())),
        }
    }
}

impl<T> Default for Given<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> From<T> for Given<T> {
    fn from(value: T) -> Self {
        Self(Some(value))
    }
}

// pydantic's Field(min_length=1): the list serializes as itself and the empty case fails where the field is read.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(try_from = "Vec<T>")]
pub struct NonEmpty<T>(Vec<T>);

#[derive(Debug, Error)]
#[error("the list must not be empty")]
pub struct EmptyList;

impl<T> TryFrom<Vec<T>> for NonEmpty<T> {
    type Error = EmptyList;

    fn try_from(items: Vec<T>) -> Result<Self, Self::Error> {
        if items.is_empty() {
            return Err(EmptyList);
        }
        Ok(Self(items))
    }
}

// pydantic's Field(default=None): absent, explicit null, or a value. serde_with's option visitor reads the inner type through the same deserializer, so its own error and path survive.
pub type Field<T> = Option<Option<T>>;

pub(crate) mod optional;
