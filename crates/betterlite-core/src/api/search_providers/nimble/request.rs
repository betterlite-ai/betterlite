use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::{PERPLEXITY_PARAMS, SearchQuery};
use indexmap::IndexMap;
use serde::Serialize;

#[derive(Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SearchBody(IndexMap<String, JsonValue>);

pub(super) fn search_body(query: &SearchQuery, optional_params: &Object) -> SearchBody {
    let mut body = IndexMap::new();
    let (include_domains, exclude_domains) =
        domain_filters(optional_params.get("search_domain_filter"));
    if !include_domains.is_empty() {
        body.insert(
            "include_domains".to_owned(),
            JsonValue::Array(include_domains.into_iter().map(JsonValue::String).collect()),
        );
    }
    if !exclude_domains.is_empty() {
        body.insert(
            "exclude_domains".to_owned(),
            JsonValue::Array(exclude_domains.into_iter().map(JsonValue::String).collect()),
        );
    }
    for (name, value) in optional_params {
        if !PERPLEXITY_PARAMS.contains(&name.as_str()) {
            body.insert(name.clone(), value.clone());
        }
    }
    // the computed query lands last, so a caller's `query` option never replaces it
    body.insert("query".to_owned(), JsonValue::String(query_text(query)));
    if let Some(max_results) = optional_params
        .get("max_results")
        .filter(|value| !value.is_null())
    {
        body.insert("max_results".to_owned(), max_results.clone());
    }
    if let Some(country) = optional_params.get("country").and_then(JsonValue::as_str) {
        body.insert(
            "country".to_owned(),
            JsonValue::String(country.to_uppercase()),
        );
    }
    SearchBody(body)
}

fn query_text(query: &SearchQuery) -> String {
    match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(parts) => parts.join(" "),
    }
}

// a unary '-' marks an excluded domain; empty entries and a bare '-' are dropped
fn domain_filters(value: Option<&JsonValue>) -> (Vec<String>, Vec<String>) {
    let Some(JsonValue::Array(entries)) = value else {
        return (Vec::new(), Vec::new());
    };
    let Some(domains) = entries
        .iter()
        .map(JsonValue::as_str)
        .collect::<Option<Vec<&str>>>()
    else {
        return (Vec::new(), Vec::new());
    };
    let include_domains = domains
        .iter()
        .filter(|domain| !domain.is_empty() && !domain.starts_with('-'))
        .map(|domain| (*domain).to_owned())
        .collect();
    let exclude_domains = domains
        .iter()
        .filter_map(|domain| domain.strip_prefix('-'))
        .filter(|domain| !domain.is_empty())
        .map(|domain| domain.to_owned())
        .collect();
    (include_domains, exclude_domains)
}
