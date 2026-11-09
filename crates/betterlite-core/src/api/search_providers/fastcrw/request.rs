use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::{PERPLEXITY_PARAMS, SearchQuery};
use serde::Serialize;

#[derive(Serialize)]
pub struct SearchBody {
    query: String,
    #[serde(flatten)]
    extra: Object,
    #[serde(rename = "scrapeOptions", skip_serializing_if = "Option::is_none")]
    scrape_options: Option<ScrapeOptions>,
}

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
pub(super) fn search_body(query: &SearchQuery, optional_params: &Object) -> SearchBody {
    let mut extra = Object::new();
    if let Some(limit) = optional_params.get("max_results") {
        extra.insert("limit".to_owned(), limit.clone());
    }
    for (name, value) in optional_params {
        let reserved = name == "query" || PERPLEXITY_PARAMS.contains(&name.as_str());
        if !reserved && !extra.contains_key(name) {
            extra.insert(name.clone(), value.clone());
        }
    }
    // fastCRW returns no page content unless scrapeOptions asks for it
    let scrape_options = (!extra.contains_key("scrapeOptions")).then_some(DEFAULT_SCRAPE);
    SearchBody {
        query: query_text(query),
        extra,
        scrape_options,
    }
}

fn query_text(query: &SearchQuery) -> String {
    match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(parts) => parts.join(" "),
    }
}
