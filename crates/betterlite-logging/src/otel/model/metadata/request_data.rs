use super::common::{field, object};
use crate::otel::model::utils::coercion::OtelModelValue;
use std::collections::BTreeMap;

#[must_use]
pub fn model_from_request_data(data: &OtelModelValue) -> Option<String> {
    match field(data, "model") {
        Some(OtelModelValue::String(model)) => Some(model.to_owned()),
        Some(
            OtelModelValue::Null(())
            | OtelModelValue::Bool(_)
            | OtelModelValue::Number(_)
            | OtelModelValue::Array(_)
            | OtelModelValue::Object(_),
        )
        | None => None,
    }
}

#[must_use]
pub fn metadata_from_request_data(
    data: &OtelModelValue,
) -> Option<BTreeMap<String, OtelModelValue>> {
    ["metadata", "litellm_metadata"]
        .into_iter()
        .find_map(|name| {
            let metadata = object(field(data, name))?;
            let requester = object(metadata.get("requester_metadata"))?;
            Some(BTreeMap::from([(
                "requester_metadata".to_owned(),
                OtelModelValue::Object(requester.clone()),
            )]))
        })
}
