use super::error::RedisSemanticError;
use super::ports::IndexSchema;
use super::{CACHE_KEY_FIELD_NAME, VECTOR_FIELD_NAME};
use crate::arguments::json::{Json, Object};

const EXPECTED_FIELDS: [(&str, FieldKind); 6] = [
    ("prompt", FieldKind::Text),
    ("response", FieldKind::Text),
    ("inserted_at", FieldKind::Numeric),
    ("updated_at", FieldKind::Numeric),
    (VECTOR_FIELD_NAME, FieldKind::Vector),
    (CACHE_KEY_FIELD_NAME, FieldKind::Tag),
];

#[derive(Debug, Clone, Copy)]
enum FieldKind {
    Text,
    Numeric,
    Tag,
    Vector,
}

pub(crate) fn info_object(reply: &Json) -> Result<Json, RedisSemanticError> {
    let Json::List(items) = reply else {
        return Err(RedisSemanticError::MalformedIndexInfo);
    };
    let Some(pairs) = pair_entries(items) else {
        return Err(RedisSemanticError::MalformedIndexInfo);
    };
    Ok(Json::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), convert_info_value(value)))
            .collect(),
    ))
}

// FT.INFO answers with alternating names and values; an even-length nested array of text names is a map.
pub(crate) fn pair_entries(items: &[Json]) -> Option<Vec<(&str, &Json)>> {
    if items.len() < 2 || !items.len().is_multiple_of(2) {
        return None;
    }
    let mut pairs = Vec::new();
    for [key, value] in items.as_chunks::<2>().0 {
        let Json::Text(name) = key else {
            return None;
        };
        pairs.push((name.as_str(), value));
    }
    Some(pairs)
}

fn convert_info_value(value: &Json) -> Json {
    let Json::List(items) = value else {
        return value.clone();
    };
    pair_entries(items).map_or_else(
        || Json::List(items.iter().map(convert_info_value).collect()),
        |pairs| {
            Json::Object(
                pairs
                    .into_iter()
                    .map(|(key, item)| (key.to_owned(), convert_info_value(item)))
                    .collect(),
            )
        },
    )
}

pub(crate) fn schema_matches(reply: &Json, schema: &IndexSchema) -> bool {
    let Ok(Json::Object(info)) = info_object(reply) else {
        return false;
    };
    let Some(Json::Object(definition)) = info.get("index_definition") else {
        return false;
    };
    if !text_equals(definition.get("key_type"), "hash") {
        return false;
    }
    if !prefix_matches(definition.get("prefixes"), &schema.prefix) {
        return false;
    }
    let Some(Json::List(attributes)) = info.get("attributes") else {
        return false;
    };
    attributes.len() == EXPECTED_FIELDS.len()
        && attributes
            .iter()
            .zip(EXPECTED_FIELDS)
            .all(|(attribute, (name, kind))| {
                attribute_matches(attribute, name, kind, schema.dimension)
            })
}

fn text_equals(value: Option<&Json>, expected: &str) -> bool {
    let Some(text) = value.and_then(Json::as_str) else {
        return false;
    };
    text.eq_ignore_ascii_case(expected)
}

fn prefix_matches(value: Option<&Json>, prefix: &str) -> bool {
    match value {
        Some(Json::List(items)) => {
            items.len() == 1 && items.first().and_then(Json::as_str) == Some(prefix)
        }
        Some(Json::Text(text)) => text == prefix,
        _ => false,
    }
}

fn attribute_matches(attribute: &Json, name: &str, kind: FieldKind, dimension: usize) -> bool {
    let Json::Object(fields) = attribute else {
        return false;
    };
    let actual_name = fields
        .get("identifier")
        .or_else(|| fields.get("attribute"))
        .and_then(Json::as_str);
    if actual_name != Some(name) {
        return false;
    }
    match kind {
        FieldKind::Text => text_equals(fields.get("type"), "text"),
        FieldKind::Numeric => text_equals(fields.get("type"), "numeric"),
        FieldKind::Tag => text_equals(fields.get("type"), "tag"),
        FieldKind::Vector => {
            text_equals(fields.get("type"), "vector") && vector_attrs_match(fields, dimension)
        }
    }
}

fn vector_attrs_match(fields: &Object, dimension: usize) -> bool {
    let dimension_value = fields
        .get("dim")
        .or_else(|| fields.get("dims"))
        .and_then(json_usize);
    let datatype = fields
        .get("data_type")
        .or_else(|| fields.get("datatype"))
        .and_then(Json::as_str);
    dimension_value == Some(dimension)
        && text_equals(fields.get("algorithm"), "flat")
        && text_equals(fields.get("distance_metric"), "cosine")
        && datatype.is_some_and(|value| value.eq_ignore_ascii_case("float32"))
}

fn json_usize(value: &Json) -> Option<usize> {
    match value {
        Json::Number(raw) => {
            let count = raw.as_u64()?;
            usize::try_from(count).ok()
        }
        Json::Text(text) => text.parse::<usize>().ok(),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}
