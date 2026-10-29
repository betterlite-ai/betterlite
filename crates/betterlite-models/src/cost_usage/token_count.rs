use serde::Deserialize;
use serde::de::IgnoredAny;

// Reads with Python's isinstance(value, int) leniency: a bool counts, anything else counts as 0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(from = "RawTokenCount")]
pub(super) struct TokenCount(pub(super) i64);

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawTokenCount {
    Bool(bool),
    Int(Box<i64>),
    Other(Box<IgnoredAny>),
}

impl From<RawTokenCount> for TokenCount {
    fn from(raw: RawTokenCount) -> Self {
        match raw {
            RawTokenCount::Bool(value) => Self(i64::from(value)),
            RawTokenCount::Int(value) => Self(*value),
            RawTokenCount::Other(_ignored) => Self(0),
        }
    }
}

mod tests;
