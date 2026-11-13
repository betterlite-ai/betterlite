use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use url::{Url, form_urlencoded};

pub type QueryParameters = IndexMap<String, QueryValue>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum QueryValue {
    Single(String),
    Multiple(Vec<String>),
}

#[must_use]
pub fn decode_request_query(raw: &str) -> IndexMap<String, Vec<String>> {
    let mut decoded: IndexMap<String, Vec<String>> = IndexMap::new();
    for (name, value) in form_urlencoded::parse(raw.as_bytes()) {
        // parse_qs drops a blank value, and its default separator is only '&'
        if value.is_empty() {
            continue;
        }
        decoded
            .entry(name.into_owned())
            .or_default()
            .push(value.into_owned());
    }
    decoded
}

#[must_use]
pub fn merge_query_parameters(
    existing_url: &Url,
    request_query_params: &QueryParameters,
    default_query_params: Option<&QueryParameters>,
) -> QueryParameters {
    let mut merged: QueryParameters = default_query_params.cloned().unwrap_or_default();
    let query = existing_url.query().unwrap_or_default();
    for (name, mut values) in decode_request_query(query) {
        let value = if values.len() == 1
            && let Some(single) = values.pop()
        {
            QueryValue::Single(single)
        } else {
            QueryValue::Multiple(values)
        };
        merged.insert(name, value);
    }
    for (name, value) in request_query_params {
        merged.insert(name.clone(), value.clone());
    }
    merged
}

mod tests;
