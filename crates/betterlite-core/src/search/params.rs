use betterlite_context::owned_parameter_names::is_litellm_owned_kwarg;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde_json::Number;

use super::SearchOptions;

// search() pops the logging object and asearch's flag out of kwargs before anything reads them
const POPPED_KWARGS: [&str; 2] = ["litellm_logging_obj", "asearch"];

pub(super) fn call_kwargs(kwargs: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let mut kept = kwargs.clone();
    kept.retain(|name, _| !POPPED_KWARGS.contains(&name.as_str()));
    kept
}

pub(super) fn merged_params(
    options: &SearchOptions,
    kwargs: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    if let Some(max_results) = options.max_results {
        params.insert(
            "max_results".to_owned(),
            Json::Number(Number::from(max_results)),
        );
    }
    if let Some(domains) = &options.search_domain_filter {
        params.insert(
            "search_domain_filter".to_owned(),
            Json::List(domains.iter().cloned().map(Json::Text).collect()),
        );
    }
    if let Some(max_tokens) = options.max_tokens_per_page {
        params.insert(
            "max_tokens_per_page".to_owned(),
            Json::Number(Number::from(max_tokens)),
        );
    }
    if let Some(country) = &options.country {
        params.insert("country".to_owned(), Json::Text(country.clone()));
    }
    for (name, value) in kwargs {
        if !is_litellm_owned_kwarg(name) && !params.contains_key(name) {
            params.insert(name.clone(), value.clone());
        }
    }
    params
}

pub(super) fn provider_params(
    params: &IndexMap<String, Json>,
) -> Result<Object, serde_json::Error> {
    params
        .iter()
        .map(|(name, value)| serde_json::to_value(value).map(|encoded| (name.clone(), encoded)))
        .collect()
}
