use std::collections::BTreeMap;

use betterlite_azure::azure_urls::url_parts::{encoded_query, query_pairs};
use indexmap::IndexMap;

// httpx's QueryParams keeps every value of a repeated name, in first-seen order
#[must_use]
pub fn merged_query(query: Option<&str>, configured: &BTreeMap<String, String>) -> Option<String> {
    let mut groups: IndexMap<String, Vec<String>> = IndexMap::new();
    for (key, value) in query_pairs(query) {
        groups.entry(key).or_default().push(value);
    }
    // IndexMap::insert keeps the position of a key the URL already has, as QueryParams.merge does
    for (key, value) in configured {
        groups.insert(key.clone(), vec![value.clone()]);
    }
    let mut pairs: Vec<(String, String)> = Vec::new();
    for (key, values) in groups {
        for value in values {
            pairs.push((key.clone(), value));
        }
    }
    encoded_query(&pairs)
}
