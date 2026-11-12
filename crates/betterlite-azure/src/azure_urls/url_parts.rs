use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use percent_encoding::percent_decode_str;
use url::Url;
use url::form_urlencoded;

use super::AzureUrlError;

pub(super) fn path_decoded(url: &Url) -> Result<String, AzureUrlError> {
    percent_decode_str(url.path())
        .decode_utf8()
        .map(Cow::into_owned)
        .map_err(AzureUrlError::InvalidPath)
}

pub(super) fn query_pairs(url: &Url) -> Vec<(String, String)> {
    url.query_pairs()
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect()
}

// httpx's dict(URL.params) keeps the first value of each name in first-occurrence order
pub(super) fn first_query_values(url: &Url) -> Vec<(String, String)> {
    let mut order = Vec::new();
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in url.query_pairs() {
        let owned = name.into_owned();
        if let Entry::Vacant(entry) = values.entry(owned.clone()) {
            order.push(owned);
            entry.insert(value.into_owned());
        }
    }
    let mut kept = Vec::with_capacity(order.len());
    for name in order {
        if let Some(value) = values.remove(&name) {
            kept.push((name, value));
        }
    }
    kept
}

pub(super) fn set_query_pairs(url: &mut Url, pairs: &[(String, String)]) {
    if pairs.is_empty() {
        url.set_query(None);
        return;
    }
    let query = encode_query(pairs);
    url.set_query(Some(&query));
}

// url::Url renders an empty path as "/", while httpx keeps an absolute URL's authority followed by nothing
pub(super) fn has_empty_path(raw: &str) -> bool {
    let before_fragment = raw.split_once('#').map_or(raw, |(head, _fragment)| head);
    let before_query = before_fragment
        .split_once('?')
        .map_or(before_fragment, |(head, _query)| head);
    let Some((_scheme, rest)) = before_query.split_once("://") else {
        return false;
    };
    !rest.contains('/')
}

pub(super) fn without_empty_path(text: &str) -> String {
    let Some(scheme_end) = text.find("://") else {
        return text.to_owned();
    };
    let authority_start = scheme_end.saturating_add(3);
    let Some(offset) = text.get(authority_start..).and_then(|rest| rest.find('/')) else {
        return text.to_owned();
    };
    let slash = authority_start.saturating_add(offset);
    let tail = text.get(slash.saturating_add(1)..).unwrap_or_default();
    if !tail.is_empty() && !tail.starts_with('?') && !tail.starts_with('#') {
        return text.to_owned();
    }
    let head = text.get(..slash).unwrap_or_default();
    format!("{head}{tail}")
}

fn encode_query(pairs: &[(String, String)]) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (name, value) in pairs {
        serializer.append_pair(name, value);
    }
    // Python's urlencode uses quote_plus: it encodes * and keeps ~, the reverse of form_urlencoded
    serializer.finish().replace('*', "%2A").replace("%7E", "~")
}
