use super::error::RedisSemanticError;
use super::ports::IndexSchema;
use crate::arguments::json::{Json, Object};

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
            .map(|(key, value)| (key.to_owned(), info_value(value)))
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

fn info_value(value: &Json) -> Json {
    let Json::List(items) = value else {
        return value.clone();
    };
    pair_entries(items).map_or_else(
        || Json::List(items.iter().map(info_value).collect()),
        |pairs| {
            Json::Object(
                pairs
                    .into_iter()
                    .map(|(key, item)| (key.to_owned(), info_value(item)))
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
    let Some(Json::List(attributes)) = info.get("attributes") else {
        return false;
    };
    let signatures: Option<Vec<String>> = attributes.iter().map(attribute_signature).collect();
    let expected = [
        "PROMPT:TEXT".to_owned(),
        "RESPONSE:TEXT".to_owned(),
        "INSERTED_AT:NUMERIC".to_owned(),
        "UPDATED_AT:NUMERIC".to_owned(),
        format!(
            "PROMPT_VECTOR:VECTOR:{}:FLAT:FLOAT32:COSINE",
            schema.dimension
        ),
        "LITELLM_CACHE_KEY:TAG".to_owned(),
    ];
    signatures.is_some_and(|found| found == expected)
        && text_equals(definition.get("key_type"), "hash")
        && prefix_matches(definition.get("prefixes"), &schema.prefix)
}

fn attribute_signature(attribute: &Json) -> Option<String> {
    let Json::Object(fields) = attribute else {
        return None;
    };
    let name = field_text(fields, &["identifier", "attribute"])?;
    let kind = field_text(fields, &["type"])?;
    if !kind.eq_ignore_ascii_case("vector") {
        return Some(format!("{name}:{kind}").to_uppercase());
    }
    let dims = field_usize(fields, &["dim", "dims"])?;
    let datatype = field_text(fields, &["data_type", "datatype"])?;
    let algorithm = field_text(fields, &["algorithm"])?;
    let metric = field_text(fields, &["distance_metric"])?;
    Some(format!("{name}:VECTOR:{dims}:{algorithm}:{datatype}:{metric}").to_uppercase())
}

fn field_text<'field>(fields: &'field Object, names: &[&str]) -> Option<&'field str> {
    names
        .iter()
        .find_map(|name| fields.get(*name).and_then(Json::as_str))
}

fn field_usize(fields: &Object, names: &[&str]) -> Option<usize> {
    names.iter().find_map(|name| match fields.get(*name) {
        Some(Json::Number(raw)) => {
            let count = raw.as_u64()?;
            usize::try_from(count).ok()
        }
        Some(Json::Text(text)) => text.parse().ok(),
        _ => None,
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
