use std::collections::BTreeMap;

use serde::Deserialize;

use crate::tiered_pricing::Scalar;

// An untagged enum buffers the input and tries Mapping(T) first; T must itself reject array input (see each T's own map-backed From), or a same-arity array would fill its fields positionally instead of falling through to Other.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum MappingOrScalar<T> {
    Mapping(T),
    Other(Scalar),
}

impl<T> MappingOrScalar<T> {
    #[must_use]
    pub const fn as_mapping(&self) -> Option<&T> {
        match self {
            Self::Mapping(mapping) => Some(mapping),
            Self::Other(_) => None,
        }
    }
}

// The one field shape in this module that needs a level of structure preserved instead of collapsing to Scalar.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum MapValue {
    Mapping(BTreeMap<String, Scalar>),
    Scalar(Scalar),
}

mod tests;
