use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Field<T> {
    #[default]
    #[serde(skip)]
    Absent,
    Present(Option<T>),
}

impl<T> Field<T> {
    #[must_use]
    pub const fn is_absent(&self) -> bool {
        match self {
            Self::Absent => true,
            Self::Present(_) => false,
        }
    }
}
