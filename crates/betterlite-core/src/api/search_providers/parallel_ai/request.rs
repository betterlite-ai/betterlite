use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::SearchQuery;
use indexmap::IndexMap;
use serde::Serialize;

use super::error::ParallelAiSearchError;

pub const PARALLEL_AI_USAGE_PARAM: &str = "_parallel_ai_usage";

#[derive(Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SearchBody(IndexMap<String, JsonValue>);

pub(super) fn search_body(
    query: &SearchQuery,
    optional_params: &Object,
) -> Result<SearchBody, ParallelAiSearchError> {
    let mut params = optional_params.clone();
    let mut body = IndexMap::new();
    match query {
        SearchQuery::Single(text) => {
            body.insert(
                "search_queries".to_owned(),
                JsonValue::Array(vec![JsonValue::String(text.clone())]),
            );
            body.insert("objective".to_owned(), JsonValue::String(text.clone()));
        }
        SearchQuery::Multiple(parts) => {
            body.insert(
                "search_queries".to_owned(),
                JsonValue::Array(parts.iter().cloned().map(JsonValue::String).collect()),
            );
        }
    }
    body.insert("mode".to_owned(), mode(&mut params));
    let settings = advanced_settings(&mut params)?;
    if !settings.is_empty() {
        body.insert("advanced_settings".to_owned(), JsonValue::Object(settings));
    }
    // the remaining options merge last: one named search_queries or objective overrides the derived value
    for (name, value) in params {
        if name != "max_tokens_per_page" && name != PARALLEL_AI_USAGE_PARAM {
            body.insert(name, value);
        }
    }
    Ok(SearchBody(body))
}

// a mode wins over the legacy processor, and an empty mode keeps v1beta's basic tier
fn mode(params: &mut Object) -> JsonValue {
    let caller_mode = params.remove("mode").filter(|value| !value.is_null());
    let legacy_processor = params.remove("processor").filter(|value| !value.is_null());
    let resolved_mode = match caller_mode {
        Some(chosen_mode) => chosen_mode,
        None => legacy_processor.map_or_else(
            || JsonValue::String("basic".to_owned()),
            |processor| match processor.as_str() {
                Some("base") => JsonValue::String("basic".to_owned()),
                Some("pro") => JsonValue::String("advanced".to_owned()),
                _ => processor,
            },
        ),
    };
    match resolved_mode {
        JsonValue::String(text) if text.is_empty() => JsonValue::String("basic".to_owned()),
        other => other,
    }
}

fn advanced_settings(params: &mut Object) -> Result<Object, ParallelAiSearchError> {
    let mut settings = Object::new();
    if let Some(max_results) = params.remove("max_results") {
        settings.insert("max_results".to_owned(), max_results);
    }
    // the unified country becomes the v1 location, and an explicit location replaces it
    if let Some(country) = params.remove("country") {
        settings.insert("location".to_owned(), country);
    }
    if let Some(location) = params.remove("location") {
        settings.insert("location".to_owned(), location);
    }
    if let Some(max_chars) = params.remove("max_chars_per_result") {
        let mut excerpt_settings = Object::new();
        excerpt_settings.insert("max_chars_per_result".to_owned(), max_chars);
        settings.insert(
            "excerpt_settings".to_owned(),
            JsonValue::Object(excerpt_settings),
        );
    }
    if let Some(fetch_policy) = params.remove("fetch_policy") {
        settings.insert("fetch_policy".to_owned(), fetch_policy);
    }
    let mut source_policy = Object::new();
    if let Some(domains) = params.remove("search_domain_filter") {
        source_policy.insert("include_domains".to_owned(), domains);
    }
    if let Some(domains) = params.remove("include_domains") {
        source_policy.insert("include_domains".to_owned(), domains);
    }
    if let Some(domains) = params.remove("exclude_domains") {
        source_policy.insert("exclude_domains".to_owned(), domains);
    }
    if let Some(after_date) = params.remove("after_date") {
        source_policy.insert("after_date".to_owned(), after_date);
    }
    if !source_policy.is_empty() {
        settings.insert("source_policy".to_owned(), JsonValue::Object(source_policy));
    }
    match params.remove("advanced_settings") {
        None | Some(JsonValue::Null) => {}
        Some(JsonValue::Object(caller)) => {
            for (name, value) in caller {
                settings.insert(name, value);
            }
        }
        Some(_) => {
            return Err(ParallelAiSearchError::OptionType {
                field: "advanced_settings",
            });
        }
    }
    Ok(settings)
}
