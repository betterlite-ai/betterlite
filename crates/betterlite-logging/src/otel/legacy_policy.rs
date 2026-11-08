use std::collections::BTreeMap;

use crate::otel::model::utils::coercion::OtelModelValue;

pub mod annotations;
pub mod capture;
pub mod config;
pub mod dedupe;
pub mod error_recording;
pub mod semconv_attrs;
pub mod span_time;

pub(crate) fn text_field<'source>(
    fields: &'source BTreeMap<String, OtelModelValue>,
    key: &str,
) -> Option<&'source str> {
    fields
        .get(key)
        .and_then(OtelModelValue::as_str)
        .filter(|text| !text.is_empty())
}
