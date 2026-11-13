use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::{PERPLEXITY_PARAMS, SearchQuery};

use super::{DefaultContents, ExaAiSearchRequest};

pub(super) fn build(query: &SearchQuery, optional_params: &Object) -> ExaAiSearchRequest {
    let joined = match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(queries) => queries.join(" "),
    };
    let mut fields = Object::new();
    for (field, unified) in [
        ("numResults", "max_results"),
        ("includeDomains", "search_domain_filter"),
        ("userLocation", "country"),
    ] {
        if let Some(value) = optional_params.get(unified) {
            fields.insert(field.to_owned(), value.clone());
        }
    }
    for (name, value) in optional_params {
        if PERPLEXITY_PARAMS.contains(&name.as_str()) {
            continue;
        }
        if is_already_in_body(name, optional_params) {
            continue;
        }
        fields.insert(name.clone(), value.clone());
    }
    // a caller contents of {} stays {}, so only an absent key gets the default
    let default_contents =
        (!optional_params.contains_key("contents")).then_some(DefaultContents { text: true });
    ExaAiSearchRequest {
        query: joined,
        default_contents,
        fields,
    }
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
