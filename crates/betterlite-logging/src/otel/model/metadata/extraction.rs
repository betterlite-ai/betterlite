use super::wire::{
    MetadataMap,
    callback::{Callback, RequestData},
    payload::Payload,
};
use crate::otel::model::utils::coercion::OtelModelValue;
use std::collections::BTreeMap;

#[must_use]
pub fn metadata_from_request_data(data: &RequestData) -> Option<MetadataMap> {
    [&data.metadata, &data.litellm_metadata]
        .into_iter()
        .find_map(|metadata| {
            Some(BTreeMap::from([(
                "requester_metadata".to_owned(),
                OtelModelValue::Object(metadata.requester_metadata.clone()?),
            )]))
        })
}

#[must_use]
pub fn auth_metadata(
    payload: Option<&Payload>,
    kwargs: &Callback,
) -> Option<BTreeMap<String, String>> {
    [
        payload.map(|value| &value.metadata),
        Some(&kwargs.litellm_params.metadata),
        Some(&kwargs.litellm_params.litellm_metadata),
    ]
    .into_iter()
    .flatten()
    .filter_map(|metadata| metadata.user_api_key_auth_metadata.as_ref())
    .find_map(|entries| {
        let typed: BTreeMap<String, String> = entries
            .iter()
            .filter_map(|(key, value)| value.as_str().map(|text| (key.clone(), text.to_owned())))
            .collect();
        (!typed.is_empty()).then_some(typed)
    })
}

mod tests;
