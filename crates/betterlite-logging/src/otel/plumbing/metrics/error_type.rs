use super::attributes::dump;
use crate::otel::legacy_metrics::error::LegacyMetricsError;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;

// the convention's sentinel for a failure the instrumentation cannot classify
pub const ERROR_TYPE_FALLBACK: &str = "_OTHER";

// kwargs carries the exception as an object with class_name, the shape betterlite's exception facts serialize to
pub fn resolve_error_type(kwargs: &MetadataMap) -> Result<String, LegacyMetricsError> {
    let information = kwargs
        .get("standard_logging_object")
        .and_then(OtelModelValue::as_object)
        .and_then(|payload| payload.get("error_information"))
        .and_then(OtelModelValue::as_object);
    if let Some(information) = information {
        for field in ["error_class", "error_code"] {
            let Some(value) = information.get(field) else {
                continue;
            };
            if matches!(value, OtelModelValue::Null(())) {
                continue;
            }
            return text(value);
        }
    }
    let Some(exception) = kwargs.get("exception") else {
        return Ok(ERROR_TYPE_FALLBACK.to_owned());
    };
    let class_name = exception
        .as_object()
        .and_then(|fields| fields.get("class_name"))
        .and_then(OtelModelValue::as_str)
        .filter(|name| !name.is_empty());
    Ok(class_name.map_or_else(|| ERROR_TYPE_FALLBACK.to_owned(), str::to_owned))
}

fn text(value: &OtelModelValue) -> Result<String, LegacyMetricsError> {
    match value.as_str() {
        Some(text) => Ok(text.to_owned()),
        None => dump(value),
    }
}
