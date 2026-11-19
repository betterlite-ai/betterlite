use std::collections::BTreeMap;

use indexmap::IndexMap;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use url::Url;

// Python's quote_plus leaves alphanumerics and _.-~ alone and writes a space as a plus
const QUOTE_PLUS: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'-')
    .remove(b'.')
    .remove(b'~');

#[must_use]
pub fn merged_query(url: &Url, configured: &BTreeMap<String, String>) -> Option<String> {
    let mut groups: IndexMap<String, Vec<String>> = IndexMap::new();
    for (key, value) in url.query_pairs() {
        groups
            .entry(key.into_owned())
            .or_default()
            .push(value.into_owned());
    }
    // IndexMap::insert keeps the position of a key the URL already has, as httpx's dict merge does
    for (key, value) in configured {
        groups.insert(key.clone(), vec![value.clone()]);
    }
    if groups.is_empty() {
        return None;
    }
    let mut encoded = String::new();
    let mut written = false;
    for (key, group) in &groups {
        for item in group {
            if written {
                encoded.push('&');
            }
            push_quote_plus(&mut encoded, key);
            encoded.push('=');
            push_quote_plus(&mut encoded, item);
            written = true;
        }
    }
    Some(encoded)
}

fn push_quote_plus(encoded: &mut String, value: &str) {
    for part in utf8_percent_encode(value, QUOTE_PLUS) {
        if part == "%20" {
            encoded.push('+');
        } else {
            encoded.push_str(part);
        }
    }
}
