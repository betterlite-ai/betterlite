use betterlite_models::decisions::Json;

use crate::gemini_metadata::extension::{MetadataExtension, MetadataField};

#[must_use]
pub fn stream_chunk_attr<'chunk>(chunk: &'chunk Json, field: &str) -> Option<&'chunk Json> {
    let Json::Object(fields) = chunk else {
        return None;
    };
    if let Some(value) = fields.get(field).filter(|value| **value != Json::Null) {
        return Some(value);
    }
    if let Some(Json::Object(model_extra)) = fields.get("model_extra")
        && let Some(value) = model_extra.get(field).filter(|value| **value != Json::Null)
    {
        return Some(value);
    }
    match fields.get("_hidden_params") {
        Some(Json::Object(hidden_params)) => hidden_params
            .get(field)
            .filter(|value| **value != Json::Null),
        Some(_) | None => None,
    }
}

#[must_use]
pub fn assemble_stream_metadata(chunks: &[Json]) -> MetadataExtension {
    let mut extension = MetadataExtension::default();
    for field in MetadataField::ALL {
        for chunk in chunks {
            let Some(value) = stream_chunk_attr(chunk, field.wire_name()) else {
                continue;
            };
            if is_falsey(value) {
                continue;
            }
            if let Json::List(entries) = value {
                extension.values_mut(field).extend(entries.iter().cloned());
            } else {
                extension.values_mut(field).push(value.clone());
            }
        }
    }
    extension
}

fn is_falsey(value: &Json) -> bool {
    match value {
        Json::Null | Json::Bool(false) => true,
        Json::Bool(true) => false,
        Json::Number(number) => number.as_f64().is_some_and(|parsed| parsed == 0.0_f64),
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(entries) => entries.is_empty(),
    }
}
