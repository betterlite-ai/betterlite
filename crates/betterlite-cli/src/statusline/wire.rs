use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;

// A field of the wrong type reads as absent, the way an untrusted payload is only trusted for what it declares
#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum Wire<T> {
    Value(T),
    Null,
    Other(IgnoredAny),
}
impl Wire<Number> {
    pub(crate) fn amount(&self) -> Option<Decimal> {
        match self {
            Self::Value(number) => decimal(number),
            Self::Null | Self::Other(_) => None,
        }
    }
    pub(crate) fn count(&self) -> Option<u64> {
        match self {
            Self::Value(number) => number.as_u64(),
            Self::Null | Self::Other(_) => None,
        }
    }
}

pub(crate) fn text(field: Option<&Wire<String>>) -> &str {
    match field {
        Some(Wire::Value(value)) => value,
        Some(Wire::Null | Wire::Other(_)) | None => "",
    }
}

// Only a JSON number counts: a string, a boolean or null is not money, and one too large for Decimal is not finite
pub(crate) fn decimal(number: &Number) -> Option<Decimal> {
    let literal = number.to_string();
    literal
        .parse::<Decimal>()
        .or_else(|_| Decimal::from_scientific(&literal))
        .ok()
}
