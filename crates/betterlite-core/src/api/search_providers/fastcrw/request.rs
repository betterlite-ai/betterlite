use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::{PERPLEXITY_PARAMS, SearchQuery};
use serde::Serialize;

use super::error::FastcrwError;

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

// LiteLLM reshapes every provider-neutral parameter and then copies the rest through untouched
pub(super) fn search_body(
    query: &SearchQuery,
    optional_params: &Object,
) -> Result<Object, FastcrwError> {
    let text = match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(parts) => parts.join(" "),
    };
    let mut body = Object::new();
    body.insert(
        "query".to_owned(),
        serde_json::to_value(text).map_err(|source| FastcrwError::Request { source })?,
    );
    if let Some(limit) = optional_params.get("max_results") {
        body.insert("limit".to_owned(), limit.clone());
    }
    for (name, value) in optional_params {
        if !PERPLEXITY_PARAMS.contains(&name.as_str()) && !body.contains_key(name) {
            body.insert(name.clone(), value.clone());
        }
    }
    if !body.contains_key("scrapeOptions") {
        body.insert(
            "scrapeOptions".to_owned(),
            serde_json::to_value(DEFAULT_SCRAPE)
                .map_err(|source| FastcrwError::Request { source })?,
        );
    }
    Ok(body)
}
