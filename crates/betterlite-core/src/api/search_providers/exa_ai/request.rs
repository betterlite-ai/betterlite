use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::{PERPLEXITY_PARAMS, SearchQuery};
use serde::Serialize;
use serde_json::value::{RawValue, to_raw_value};

use super::{ExaAiSearchError, ExaAiSearchRequest};

#[derive(Debug, Serialize)]
struct DefaultContents {
    text: bool,
}

pub(super) fn build(
    query: &SearchQuery,
    optional_params: &Object,
) -> Result<ExaAiSearchRequest, ExaAiSearchError> {
    let joined = match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(queries) => queries.join(" "),
    };
    let mut extra = Object::new();
    for (name, value) in optional_params {
        if PERPLEXITY_PARAMS.contains(&name.as_str()) || name == "contents" {
            continue;
        }
        if is_already_in_body(name, optional_params) {
            continue;
        }
        extra.insert(name.clone(), value.clone());
    }
    let contents = match raw_field(optional_params, "contents")? {
        Some(contents) => contents,
        None => {
            to_raw_value(&DefaultContents { text: true }).map_err(ExaAiSearchError::RequestJson)?
        }
    };
    Ok(ExaAiSearchRequest {
        query: joined,
        num_results: raw_field(optional_params, "max_results")?,
        include_domains: raw_field(optional_params, "search_domain_filter")?,
        user_location: raw_field(optional_params, "country")?,
        contents,
        extra,
    })
}

fn raw_field(
    optional_params: &Object,
    name: &str,
) -> Result<Option<Box<RawValue>>, ExaAiSearchError> {
    optional_params
        .get(name)
        .map(|value| to_raw_value(value).map_err(ExaAiSearchError::RequestJson))
        .transpose()
}

// the unified name only fills its Exa field when it is present, and the copy check runs after that
fn is_already_in_body(name: &str, optional_params: &Object) -> bool {
    let source = match name {
        "query" => return true,
        "numResults" => "max_results",
        "includeDomains" => "search_domain_filter",
        "userLocation" => "country",
        _ => return false,
    };
    optional_params.contains_key(source)
}
