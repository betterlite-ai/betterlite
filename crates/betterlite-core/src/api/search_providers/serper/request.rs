use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::{PERPLEXITY_PARAMS, SearchQuery};
use serde_json::value::{RawValue, to_raw_value};

use super::{SerperSearchError, SerperSearchRequest};

pub(super) fn build(
    query: &SearchQuery,
    optional_params: &Object,
) -> Result<SerperSearchRequest, SerperSearchError> {
    let joined = match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(queries) => queries.join(" "),
    };
    let mut body = SerperSearchRequest {
        query: joined,
        num: raw_field(optional_params, "max_results")?,
        gl: serper_country(optional_params)?,
        extra: Object::new(),
    };
    if let Some(clauses) = serper_domains(optional_params)? {
        body.query = format!("({}) ({})", body.query, clauses.join(" OR "));
    }
    let mut extra = Object::new();
    for (name, value) in optional_params {
        if PERPLEXITY_PARAMS.contains(&name.as_str()) {
            continue;
        }
        if serper_already_in_body(name, optional_params) {
            continue;
        }
        extra.insert(name.clone(), value.clone());
    }
    body.extra = extra;
    Ok(body)
}

fn raw_field(
    optional_params: &Object,
    name: &str,
) -> Result<Option<Box<RawValue>>, SerperSearchError> {
    optional_params
        .get(name)
        .map(|value| to_raw_value(value).map_err(SerperSearchError::RequestJson))
        .transpose()
}

fn serper_country(optional_params: &Object) -> Result<Option<String>, SerperSearchError> {
    let Some(value) = optional_params.get("country") else {
        return Ok(None);
    };
    value.as_str().map_or_else(
        || Err(SerperSearchError::OptionType { field: "country" }),
        |country| Ok(Some(country.to_lowercase())),
    )
}

fn serper_domains(optional_params: &Object) -> Result<Option<Vec<String>>, SerperSearchError> {
    let Some(value) = optional_params.get("search_domain_filter") else {
        return Ok(None);
    };
    let Some(domains) = value.as_array() else {
        return Ok(None);
    };
    if domains.is_empty() {
        return Ok(None);
    }
    let mut clauses = Vec::with_capacity(domains.len());
    for member in domains {
        let Some(domain) = member.as_str() else {
            return Err(SerperSearchError::OptionList {
                field: "search_domain_filter",
            });
        };
        clauses.push(format!("site:{domain}"));
    }
    Ok(Some(clauses))
}

// the unified name only fills its Serper field when it is present, and the copy check runs after that
fn serper_already_in_body(name: &str, optional_params: &Object) -> bool {
    let source = match name {
        "q" => return true,
        "num" => "max_results",
        "gl" => "country",
        _ => return false,
    };
    optional_params.contains_key(source)
}
