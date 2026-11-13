use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;

// the Langfuse projections read betterlite's ordered Json; the OTel primitives read OtelModelValue
pub(crate) fn json_value(value: &OtelModelValue) -> Json {
    match value {
        OtelModelValue::Null(()) => Json::Null,
        OtelModelValue::Bool(flag) => Json::Bool(*flag),
        OtelModelValue::Number(number) => Json::Number(number.clone()),
        OtelModelValue::String(text) => Json::Text(text.clone()),
        OtelModelValue::Array(items) => Json::List(items.iter().map(json_value).collect()),
        OtelModelValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), json_value(item)))
                .collect(),
        ),
    }
}

// the two value types are the same JSON tree, so serde carries every number and string exactly
pub(crate) fn model_value(value: &Json) -> OtelModelValue {
    let Ok(encoded) = serde_json::to_string(value) else {
        return OtelModelValue::Null(());
    };
    serde_json::from_str(&encoded).unwrap_or(OtelModelValue::Null(()))
}

pub(crate) fn json_from_metadata(kwargs: &MetadataMap) -> Json {
    let mut fields = IndexMap::new();
    for (key, value) in kwargs {
        fields.insert(key.clone(), json_value(value));
    }
    Json::Object(fields)
}

#[must_use]
pub(crate) const fn object(value: &Json) -> Option<&IndexMap<String, Json>> {
    let Json::Object(fields) = value else {
        return None;
    };
    Some(fields)
}

#[must_use]
pub(crate) fn field<'value>(value: &'value Json, name: &str) -> Option<&'value Json> {
    let Json::Object(fields) = value else {
        return None;
    };
    fields.get(name)
}

#[must_use]
pub(crate) fn array(value: &Json) -> Option<&[Json]> {
    let Json::List(items) = value else {
        return None;
    };
    Some(items)
}

#[must_use]
pub(crate) fn text(value: &Json) -> Option<&str> {
    let Json::Text(text) = value else {
        return None;
    };
    Some(text)
}
