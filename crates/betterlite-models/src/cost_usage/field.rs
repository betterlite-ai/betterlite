use serde::Deserialize;

// A key set to null is still present, so a plain Option would lose it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub(super) enum Field<T> {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Present(T),
}

impl<T> Field<T> {
    pub(super) const fn is_present(&self) -> bool {
        match self {
            Self::Present(_) => true,
            Self::Absent => false,
        }
    }
}
