mod tests;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use crate::oauth_resource::python_url::{self, ParsedUrl};

// Python's quote_plus keeps the unreserved set and turns a space into '+', where form_urlencoded keeps '*' and encodes '~'.
const FORM_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[must_use]
pub fn append_query_params(url: &str, params: &[(String, String)]) -> String {
    let (without_fragment, fragment) = match url.split_once('#') {
        Some((before, after)) => (before, after),
        None => (url, ""),
    };
    let (base, original_query) = match without_fragment.split_once('?') {
        Some((before, after)) => (before, after),
        None => (without_fragment, ""),
    };
    let Ok(parsed) = python_url::parse(base) else {
        return url.to_owned();
    };
    let mut pairs = form_decode_pairs(original_query);
    pairs.extend(params.iter().cloned());
    let query = form_encode(&pairs);
    python_url::join(
        &parsed.scheme,
        &parsed.netloc,
        path_with_params(base, &parsed),
        &query,
        fragment,
    )
}

// urlunparse keeps a path's ';params' where ParsedUrl drops it, so the suffix after the authority is taken from the text.
fn path_with_params<'base>(base: &'base str, parsed: &ParsedUrl) -> &'base str {
    let scheme_len = if parsed.scheme.is_empty() {
        0
    } else {
        base.find(':').map_or(0, |index| index.saturating_add(1))
    };
    let after_scheme = base.get(scheme_len..).unwrap_or_default();
    let authority_len = if after_scheme.starts_with("//") {
        2_usize.saturating_add(parsed.netloc.len())
    } else {
        0
    };
    after_scheme.get(authority_len..).unwrap_or_default()
}

// Python's parse_qsl(keep_blank_values=True): an empty segment is skipped, a missing '=' leaves an empty value.
fn form_decode_pairs(query: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for segment in query.split('&').filter(|segment| !segment.is_empty()) {
        let (name, value) = match segment.split_once('=') {
            Some((name, value)) => (name, value),
            None => (segment, ""),
        };
        pairs.push((form_decode_value(name), form_decode_value(value)));
    }
    pairs
}

fn form_decode_value(text: &str) -> String {
    let plus_decoded = text.replace('+', " ");
    let decoded = percent_decode_str(&plus_decoded).decode_utf8_lossy();
    decoded.into_owned()
}

fn form_encode(pairs: &[(String, String)]) -> String {
    let mut query = String::new();
    for (key, value) in pairs {
        if !query.is_empty() {
            query.push('&');
        }
        query.push_str(&form_encode_value(key));
        query.push('=');
        query.push_str(&form_encode_value(value));
    }
    query
}

fn form_encode_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for (index, segment) in value.split(' ').enumerate() {
        if index > 0 {
            encoded.push('+');
        }
        encoded.extend(utf8_percent_encode(segment, FORM_SAFE));
    }
    encoded
}
