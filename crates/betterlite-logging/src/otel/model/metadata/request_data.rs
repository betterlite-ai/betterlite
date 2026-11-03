use super::common::{field, object};
use crate::otel::model::utils::coercion::OtelModelValue;
use std::collections::BTreeMap;

#[must_use]
pub fn model_from_request_data(data: &OtelModelValue) -> Option<String> {
    field(data, "model")
        .and_then(OtelModelValue::as_str)
        .map(str::to_owned)
}

#[must_use]
pub fn metadata_from_request_data(
    data: &OtelModelValue,
) -> Option<BTreeMap<String, OtelModelValue>> {
    for field_name in ["metadata", "litellm_metadata"] {
        let Some(request_metadata) = object(field(data, field_name)) else {
            continue;
        };
        let Some(requester_metadata) = object(request_metadata.get("requester_metadata")) else {
            continue;
        };
        return Some(BTreeMap::from([(
            "requester_metadata".to_owned(),
            OtelModelValue::Object(requester_metadata.clone()),
        )]));
    }
    None
}
