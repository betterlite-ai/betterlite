use std::fmt;

use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

/// A usage counter read with Python's `isinstance(value, int)` leniency:
/// a bool counts as 0 or 1, and anything else that is not a plain integer counts as 0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct TokenCount(pub(super) i64);

impl<'de> Deserialize<'de> for TokenCount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(TokenCountVisitor)
    }
}

struct TokenCountVisitor;

impl<'de> Visitor<'de> for TokenCountVisitor {
    type Value = TokenCount;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a token count")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
        Ok(TokenCount(i64::from(v)))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
        Ok(TokenCount(v))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
        Ok(TokenCount(i64::try_from(v).unwrap_or(0)))
    }

    fn visit_f64<E>(self, _v: f64) -> Result<Self::Value, E> {
        Ok(TokenCount(0))
    }

    fn visit_str<E>(self, _v: &str) -> Result<Self::Value, E> {
        Ok(TokenCount(0))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(TokenCount(0))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(TokenCount(0))
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(TokenCount(0))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(TokenCount(0))
    }
}

mod tests;
