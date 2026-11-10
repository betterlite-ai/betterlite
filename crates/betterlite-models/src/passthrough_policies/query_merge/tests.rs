#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use indexmap::IndexMap;
use rstest::rstest;
use url::Url;

use super::{QueryParameters, QueryValue, decode_request_query, merge_query_parameters};

fn params(fields: &[(&str, QueryValue)]) -> QueryParameters {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn raw(fields: &[(&str, &[&str])]) -> IndexMap<String, Vec<String>> {
    fields
        .iter()
        .map(|(name, values)| {
            let owned = values.iter().map(|value| (*value).to_owned()).collect();
            ((*name).to_owned(), owned)
        })
        .collect()
}

fn single(value: &str) -> QueryValue {
    QueryValue::Single(value.to_owned())
}

fn many(values: &[&str]) -> QueryValue {
    QueryValue::Multiple(values.iter().map(|value| (*value).to_owned()).collect())
}

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

#[test]
fn decodes_pairs_in_input_order() {
    let decoded = decode_request_query("b=2&a=1&b=3");
    assert_eq!(
        decoded.keys().map(String::as_str).collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert_eq!(decoded, raw(&[("b", &["2", "3"]), ("a", &["1"])]));
}

#[rstest]
#[case::empty("")]
#[case::blank_value("a=&b")]
#[case::blank_with_separators("&&a=&b&&")]
fn blank_values_disappear(#[case] input: &str) {
    assert!(decode_request_query(input).is_empty());
}

#[test]
fn an_empty_name_survives_a_non_blank_value() {
    assert_eq!(decode_request_query("=b"), raw(&[("", &["b"])]));
}

#[test]
fn plus_and_percent_escapes_are_decoded() {
    assert_eq!(
        decode_request_query("a=b+c&d=%2F%2f&e=%zz&f=%FF"),
        raw(&[
            ("a", &["b c"]),
            ("d", &["//"]),
            ("e", &["%zz"]),
            ("f", &["\u{FFFD}"])
        ])
    );
}

#[test]
fn a_semicolon_is_part_of_the_value() {
    assert_eq!(decode_request_query("a=1;b=2"), raw(&[("a", &["1;b=2"])]));
}

#[test]
fn only_the_first_equals_sign_splits_a_pair() {
    assert_eq!(decode_request_query("a=b=c"), raw(&[("a", &["b=c"])]));
}

#[test]
fn encoded_names_are_decoded_before_grouping() {
    assert_eq!(
        decode_request_query("a%5B%5D=1&a%5B%5D=2"),
        raw(&[("a[]", &["1", "2"])])
    );
}

#[test]
fn defaults_are_lowest_and_requests_are_highest() {
    let defaults = params(&[("a", single("default")), ("d", single("default"))]);
    let requests = params(&[("c", single("request")), ("e", single("request"))]);
    let merged = merge_query_parameters(
        &url("https://upstream.example/path?a=url&b=url&c=url"),
        &requests,
        Some(&defaults),
    );
    assert_eq!(
        merged,
        params(&[
            ("a", single("url")),
            ("b", single("url")),
            ("c", single("request")),
            ("d", single("default")),
            ("e", single("request")),
        ])
    );
}

#[test]
fn a_single_url_value_flattens_to_a_scalar() {
    let merged = merge_query_parameters(&url("https://upstream.example/?a=1"), &params(&[]), None);
    assert_eq!(merged, params(&[("a", single("1"))]));
}

#[test]
fn repeated_url_values_stay_a_list() {
    let merged = merge_query_parameters(
        &url("https://upstream.example/?a=1&a=2"),
        &params(&[]),
        None,
    );
    assert_eq!(merged, params(&[("a", many(&["1", "2"]))]));
}

#[test]
fn a_blank_url_value_does_not_erase_the_default() {
    let defaults = params(&[("a", single("default"))]);
    let merged = merge_query_parameters(
        &url("https://upstream.example/?a=&b=2"),
        &params(&[]),
        Some(&defaults),
    );
    assert_eq!(
        merged,
        params(&[("a", single("default")), ("b", single("2"))])
    );
}

#[test]
fn request_values_keep_their_single_or_list_shape() {
    let requests = params(&[("a", many(&["one"])), ("b", single("one"))]);
    let merged = merge_query_parameters(&url("https://upstream.example/"), &requests, None);
    assert_eq!(merged, requests);
}

#[test]
fn a_url_without_a_query_keeps_the_defaults_and_requests() {
    let defaults = params(&[("a", single("default"))]);
    let requests = params(&[("b", single("request"))]);
    let merged = merge_query_parameters(
        &url("https://upstream.example/path"),
        &requests,
        Some(&defaults),
    );
    assert_eq!(
        merged,
        params(&[("a", single("default")), ("b", single("request"))])
    );
}

#[test]
fn url_values_are_percent_decoded_before_the_merge() {
    let merged = merge_query_parameters(
        &url("https://upstream.example/?q=a%20b"),
        &params(&[]),
        None,
    );
    assert_eq!(merged, params(&[("q", single("a b"))]));
}

#[rstest]
fn replays_the_query_parsing_corpus(#[files("fuzz/corpus/query_parsing/*")] path: PathBuf) {
    let source = fs::read_to_string(path).unwrap();
    drop(decode_request_query(&source));
}
