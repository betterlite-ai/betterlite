use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::{PERPLEXITY_PARAMS, SearchQuery};
use serde::Serialize;

use super::error::FirecrawlError;

#[derive(Serialize)]
struct ScrapeOptions {
    formats: [&'static str; 1],
    #[serde(rename = "onlyMainContent")]
    only_main_content: bool,
}

const DEFAULT_SCRAPE: ScrapeOptions = ScrapeOptions {
    formats: ["markdown"],
    only_main_content: true,
};

pub(super) fn search_body(
    query: &SearchQuery,
    optional_params: &Object,
) -> Result<Object, FirecrawlError> {
    let mut body = Object::new();
    body.insert(
        "query".to_owned(),
        serde_json::to_value(query_text(query))
            .map_err(|source| FirecrawlError::Request { source })?,
    );
    if let Some(limit) = optional_params.get("max_results") {
        body.insert("limit".to_owned(), limit.clone());
    }
    if let Some(country) = optional_params.get("country") {
        body.insert("country".to_owned(), country.clone());
    }
    for (name, value) in optional_params {
        if !PERPLEXITY_PARAMS.contains(&name.as_str()) && !body.contains_key(name) {
            body.insert(name.clone(), value.clone());
        }
    }
    // a new scrape is requested with markdown when the caller sent no options of its own
    if !body.contains_key("scrapeOptions") {
        body.insert(
            "scrapeOptions".to_owned(),
            serde_json::to_value(DEFAULT_SCRAPE)
                .map_err(|source| FirecrawlError::Request { source })?,
        );
    }
    Ok(body)
}

fn query_text(query: &SearchQuery) -> String {
    match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(parts) => parts.join(" "),
    }
}
