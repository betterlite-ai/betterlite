use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::{PERPLEXITY_PARAMS, SearchQuery};
use serde_json::json;

use super::error::YouComError;

pub(super) fn search_body(
    query: &SearchQuery,
    optional_params: &Object,
) -> Result<Object, YouComError> {
    let joined = match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(parts) => parts.join(" "),
    };
    let mut body = Object::new();
    body.insert("query".to_owned(), json!(joined));
    if let Some(value) = optional_params.get("max_results") {
        body.insert("count".to_owned(), value.clone());
    }
    if let Some(value) = optional_params.get("search_domain_filter") {
        body.insert("include_domains".to_owned(), value.clone());
    }
    if let Some(value) = optional_params.get("country") {
        let Some(country) = value.as_str() else {
            return Err(YouComError::CountryNotString);
        };
        body.insert("country".to_owned(), json!(country.to_lowercase()));
    }
    for (name, value) in optional_params {
        if !PERPLEXITY_PARAMS.contains(&name.as_str()) && !body.contains_key(name) {
            body.insert(name.clone(), value.clone());
        }
    }
    Ok(body)
}
