pub mod catalog;
pub mod managed;
pub mod model;
pub mod sources;

mod tests;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;

pub const MODEL_ROUTING_HEADER_NAME: &str = "x-litellm-model";
pub const MODEL_ROUTING_ROUTE_MARKERS: [&str; 7] = [
    "/files",
    "/batches",
    "/vector_stores",
    "/skills",
    "/evals",
    "/fine_tuning",
    "/videos",
];
pub const HEADER_OR_QUERY_ROUTE_MARKERS: [&str; 4] = ["/files", "/batches", "/skills", "/evals"];
pub const QUERY_TARGET_MODEL_ROUTE_MARKERS: [&str; 3] = ["/files", "/batches", "/fine_tuning"];
pub const BODY_TARGET_MODEL_ROUTE_MARKERS: [&str; 3] = ["/files", "/batches", "/vector_stores"];
pub const COMPLETION_MODEL_ROUTE_MARKERS: [&str; 1] = ["/evals"];
pub const SESSION_MODEL_ROUTE_MARKERS: [&str; 2] = ["/realtime/client_secrets", "/realtime/calls"];
pub const MODEL_ROUTING_ID_FIELDS: [&str; 11] = [
    "file_id",
    "input_file_id",
    "output_file_id",
    "error_file_id",
    "batch_id",
    "fine_tuning_job_id",
    "training_file",
    "validation_file",
    "vector_store_id",
    "video_id",
    "character_id",
];

// str.strip() in Python also removes U+001C to U+001F, which char::is_whitespace keeps
const STRIP_SPACE: [char; 29] = [
    '\u{9}', '\u{a}', '\u{b}', '\u{c}', '\u{d}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}', ' ',
    '\u{85}', '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}',
    '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}',
    '\u{202f}', '\u{205f}', '\u{3000}',
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestedModels {
    None,
    One(String),
    Many(Vec<String>),
}

#[must_use]
pub fn route_matches_any_marker(route: &str, markers: &[&str]) -> bool {
    let normalized_route = route.to_lowercase();
    markers
        .iter()
        .any(|marker| normalized_route.contains(marker))
}

#[must_use]
pub fn route_uses_model_routing_sources(route: &str) -> bool {
    route_matches_any_marker(route, &MODEL_ROUTING_ROUTE_MARKERS)
}

#[must_use]
pub fn is_falsey(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Bool(flag) => !flag,
        Json::Number(number) => number
            .to_string()
            .parse::<Decimal>()
            .is_ok_and(|parsed| parsed.is_zero()),
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(object) => object.is_empty(),
    }
}

#[must_use]
pub fn model_text(value: &Json) -> Option<String> {
    match value {
        Json::Null => None,
        Json::Text(text) => Some(text.clone()),
        Json::Bool(true) => Some("True".to_owned()),
        Json::Bool(false) => Some("False".to_owned()),
        Json::Number(number) => Some(number.to_string()),
        // Python's str() of a container is repr text, which betterlite does not copy
        Json::List(_) | Json::Object(_) => serde_json::to_string(value).ok(),
    }
}

fn append_scalar(candidates: &mut Vec<String>, value: &Json) {
    if let Json::Text(text) = value {
        for model in text.split(',') {
            let trimmed = model.trim_matches(STRIP_SPACE.as_slice());
            if !trimmed.is_empty() {
                candidates.push(trimmed.to_owned());
            }
        }
        return;
    }
    if let Some(text) = model_text(value) {
        let trimmed = text.trim_matches(STRIP_SPACE.as_slice());
        if !trimmed.is_empty() {
            candidates.push(trimmed.to_owned());
        }
    }
}

pub fn append_model_candidates(candidates: &mut Vec<String>, value: &Json) {
    match value {
        Json::Null => {}
        Json::List(items) => {
            for item in items {
                append_scalar(candidates, item);
            }
        }
        scalar => append_scalar(candidates, scalar),
    }
}

pub fn push_model_candidates(candidates: &mut Vec<String>, models: Vec<String>) {
    let items = models.into_iter().map(Json::Text).collect();
    append_model_candidates(candidates, &Json::List(items));
}

pub fn push_optional_model(candidates: &mut Vec<String>, model: Option<String>) {
    if let Some(model) = model {
        append_model_candidates(candidates, &Json::Text(model));
    }
}

#[must_use]
pub fn dedupe_model_candidates(candidates: &[String]) -> Vec<String> {
    let mut deduped: Vec<String> = Vec::with_capacity(candidates.len());
    for model in candidates {
        if !deduped.contains(model) {
            deduped.push(model.clone());
        }
    }
    deduped
}

#[must_use]
pub fn format_model_candidates(mut candidates: Vec<String>) -> RequestedModels {
    match candidates.len() {
        0 => RequestedModels::None,
        1 => candidates
            .pop()
            .map_or(RequestedModels::None, RequestedModels::One),
        _ => RequestedModels::Many(candidates),
    }
}

#[must_use]
pub fn case_insensitive_mapping_value<'mapping>(
    mapping: Option<&'mapping JsonObject>,
    key: &str,
) -> Option<&'mapping Json> {
    let mapping = mapping.filter(|mapping| !mapping.is_empty())?;
    if let Some(value) = mapping.get(key) {
        return Some(value);
    }
    let key_lower = key.to_lowercase();
    mapping
        .iter()
        .find(|(name, _value)| name.to_lowercase() == key_lower)
        .map(|(_name, value)| value)
}
