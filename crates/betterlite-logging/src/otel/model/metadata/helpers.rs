use super::common::{field, object, scalar_text, string};
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
            match object(Some(value)) {
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
pub fn resolve_provider_model(payload: &OtelModelValue) -> Option<String> {
    string(field(payload, "metadata").and_then(|metadata| field(metadata, "deployment")))
        .or_else(|| {
            string(
                field(payload, "hidden_params")
                    .and_then(|hidden| field(hidden, "litellm_model_name")),
            )
        })
        .or_else(|| string(field(payload, "model")))
}

mod tests;
