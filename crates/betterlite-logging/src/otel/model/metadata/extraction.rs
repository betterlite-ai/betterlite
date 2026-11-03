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

#[must_use]
pub fn auth_metadata(
    payload: Option<&OtelModelValue>,
    kwargs: &OtelModelValue,
) -> Option<BTreeMap<String, String>> {
    let payload_metadata =
        payload.and_then(|payload_value| object(field(payload_value, "metadata")));
    let params = object(field(kwargs, "litellm_params"));
    [
        payload_metadata,
        params.and_then(|values| object(values.get("metadata"))),
        params.and_then(|values| object(values.get("litellm_metadata"))),
    ]
    .into_iter()
    .filter_map(|metadata| {
        let values = metadata?;
        values.get("user_api_key_auth_metadata")
    })
    .find_map(|auth_config| {
        let values = object(Some(auth_config))?;
        let typed: BTreeMap<String, String> = values
            .iter()
            .filter_map(|(key, item)| item.as_str().map(|text| (key.clone(), text.to_owned())))
            .collect();
        (!typed.is_empty()).then_some(typed)
    })
}

mod tests;
