use std::collections::BTreeMap;

use super::error::RedisSemanticError;
use super::ports::IndexSchema;
use crate::arguments::json::Json;

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
            .map(|(key, value)| (key.to_owned(), value.clone()))
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

pub(crate) fn schema_matches(reply: &Json, schema: &IndexSchema) -> bool {
    let Ok(Json::Object(info)) = info_object(reply) else {
        return false;
    };
    let Some(Json::List(definition)) = info.get("index_definition") else {
        return false;
    };
    let Some(Json::List(attributes)) = info.get("attributes") else {
        return false;
    };
    let Some(definition_pairs) = pair_entries(definition) else {
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
    let definition_value = |name: &str| {
        definition_pairs
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| *value)
    };
    signatures.is_some_and(|found| found == expected)
        && text_equals(definition_value("key_type"), "hash")
        && prefix_matches(definition_value("prefixes"), &schema.prefix)
}

// An attribute is [identifier, name, attribute, name, type, TYPE, ...extras], as RedisVL reads it.
fn attribute_signature(attribute: &Json) -> Option<String> {
    let Json::List(items) = attribute else {
        return None;
    };
    let name = items.get(1)?.as_str()?;
    let kind = items.get(5)?.as_str()?;
    if !kind.eq_ignore_ascii_case("vector") {
        return Some(format!("{name}:{kind}").to_uppercase());
    }
    if items.len() <= 6 {
        return None;
    }
    let pairs = vector_pairs(items);
    let named = |names: &[&str], default: &str| {
        names
            .iter()
            .find_map(|key| pairs.get(*key))
            .map_or_else(|| default.to_owned(), |value| value.to_lowercase())
    };
    let dimension = ["dim", "dims"]
        .iter()
        .find_map(|key| pairs.get(*key))
        .and_then(|value| value.parse::<usize>().ok())
        .or_else(|| scanned_dimension(items))?;
    let algorithm = named(&["algorithm"], "flat");
    let datatype = named(&["data_type", "datatype", "type"], "float32");
    let metric = named(&["distance_metric"], "cosine");
    Some(format!("{name}:VECTOR:{dimension}:{algorithm}:{datatype}:{metric}").to_uppercase())
}

// RedisVL parse_vector_attrs: lowercase names, plus the older "FLAT 6 TYPE ..." layout where the algorithm comes first.
fn vector_pairs(items: &[Json]) -> BTreeMap<String, String> {
    let mut pairs = BTreeMap::new();
    let mut start = 6;
    let older = items.get(6).and_then(Json::as_str).map(str::to_uppercase);
    if let Some(algorithm) = older.filter(|text| matches!(text.as_str(), "FLAT" | "HNSW")) {
        pairs.insert("algorithm".to_owned(), algorithm);
        start = 8;
    }
    for [raw_name, raw_value] in items.get(start..).unwrap_or_default().as_chunks::<2>().0 {
        if let (Some(name), Some(value)) = (raw_name.as_str(), text_of(raw_value)) {
            pairs.insert(name.to_lowercase(), value);
        }
    }
    pairs
}

fn scanned_dimension(items: &[Json]) -> Option<usize> {
    items.get(6..)?.windows(2).find_map(|window| {
        let [name, value] = window else {
            return None;
        };
        let upper = name.as_str()?.to_uppercase();
        if upper == "DIM" || upper == "DIMS" {
            text_of(value)?.parse().ok()
        } else {
            None
        }
    })
}

fn text_of(value: &Json) -> Option<String> {
    match value {
        Json::Text(text) => Some(text.clone()),
        Json::Number(number) => Some(number.to_string()),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
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
