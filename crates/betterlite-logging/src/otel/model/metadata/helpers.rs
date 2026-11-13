use super::common::scalar_text;
use super::wire::payload::Payload;
use crate::otel::model::utils::coercion::OtelModelValue;
use std::collections::BTreeMap;

#[must_use]
pub fn flatten_metadata(
    metadata: Option<&BTreeMap<String, OtelModelValue>>,
) -> BTreeMap<String, String> {
    fn collect(
        prefix: &str,
        values: &BTreeMap<String, OtelModelValue>,
        output: &mut BTreeMap<String, String>,
    ) {
        for (key, value) in values {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            match value.as_object() {
                Some(nested) => collect(&path, nested, output),
                None => output.extend(scalar_text(value).map(|text| (path, text))),
            }
        }
    }
    let mut output = BTreeMap::new();
    if let Some(metadata_fields) = metadata {
        collect("", metadata_fields, &mut output);
    }
    output
}

#[must_use]
pub fn resolve_provider_model(payload: &Payload) -> Option<String> {
    payload
        .metadata
        .deployment
        .clone()
        .or_else(|| payload.hidden_params.litellm_model_name.clone())
        .or_else(|| payload.model.clone())
}

mod tests;
