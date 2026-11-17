#![cfg(test)]

use std::collections::BTreeMap;

use super::merge_mcp_headers;

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn merge_mcp_headers_returns_none_without_headers() {
    let empty = BTreeMap::new();
    assert_eq!(merge_mcp_headers(None, None), None);
    assert_eq!(merge_mcp_headers(Some(&empty), Some(&empty)), None);
    assert_eq!(merge_mcp_headers(Some(&empty), None), None);
    assert_eq!(merge_mcp_headers(None, Some(&empty)), None);
}

#[test]
fn merge_mcp_headers_returns_one_side_as_is() {
    let extra = headers(&[("X-Extra", "one")]);
    let statics = headers(&[("X-Static", "two")]);
    assert_eq!(merge_mcp_headers(Some(&extra), None), Some(extra));
    assert_eq!(merge_mcp_headers(None, Some(&statics)), Some(statics));
}

#[test]
fn merge_mcp_headers_keeps_both_sides() {
    let extra = headers(&[("X-Extra", "one")]);
    let statics = headers(&[("X-Static", "two")]);
    assert_eq!(
        merge_mcp_headers(Some(&extra), Some(&statics)),
        Some(headers(&[("X-Extra", "one"), ("X-Static", "two")]))
    );
}

#[test]
fn merge_mcp_headers_lets_static_headers_win_on_an_equal_key() {
    let extra = headers(&[("Authorization", "caller")]);
    let statics = headers(&[("Authorization", "configured")]);
    assert_eq!(
        merge_mcp_headers(Some(&extra), Some(&statics)),
        Some(headers(&[("Authorization", "configured")]))
    );
}

#[test]
fn merge_mcp_headers_compares_keys_exactly() {
    let extra = headers(&[("Authorization", "caller")]);
    let statics = headers(&[("authorization", "configured")]);
    assert_eq!(
        merge_mcp_headers(Some(&extra), Some(&statics)),
        Some(headers(&[
            ("Authorization", "caller"),
            ("authorization", "configured")
        ]))
    );
}

#[test]
fn merge_mcp_headers_keeps_a_static_header_with_an_empty_value() {
    let extra = headers(&[("X-Extra", "one")]);
    let statics = headers(&[("X-Empty", "")]);
    assert_eq!(
        merge_mcp_headers(Some(&extra), Some(&statics)),
        Some(headers(&[("X-Extra", "one"), ("X-Empty", "")]))
    );
}
