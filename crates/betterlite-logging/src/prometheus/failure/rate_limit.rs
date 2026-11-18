use betterlite_error::rate_limit::RateLimitDimension;
use serde::Deserialize as _;
use serde::de::value::StrDeserializer;

use super::exception::LabelException;
use crate::logging_metadata::exception::validated_category;

// LiteLLM's _extract_rate_limit_labels: only a category and type the error enums know reach a label.
#[must_use]
pub fn extract_rate_limit_labels(
    exception: Option<&LabelException>,
) -> (Option<String>, Option<String>) {
    let Some(exception) = exception else {
        return (None, None);
    };
    let category = validated_category(exception.category.as_deref());
    let rate_limit_type = validated_rate_limit_type(exception.rate_limit_type.as_deref());
    (category, rate_limit_type)
}

// validate_rate_limit_type: a third-party .rate_limit_type string passes only when RateLimitType knows it.
fn validated_rate_limit_type(raw: Option<&str>) -> Option<String> {
    let value = raw?;
    RateLimitDimension::deserialize(StrDeserializer::<serde_json::Error>::new(value))
        .ok()
        .map(|_dimension| value.to_owned())
}
