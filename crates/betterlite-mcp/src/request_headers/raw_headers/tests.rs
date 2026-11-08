#![cfg(test)]

use http::StatusCode;
use rstest::rstest;

use super::{DUPLICATE_AUTHORIZATION_DETAIL, DuplicateAuthorization, RawHeaders};

fn parsed_headers(pairs: &[(&str, &str)]) -> RawHeaders {
    let bytes: Vec<(&[u8], &[u8])> = pairs
        .iter()
        .map(|(name, value)| (name.as_bytes(), value.as_bytes()))
        .collect();
    RawHeaders::from_pairs(&bytes).expect("valid headers")
}

#[rstest]
#[case::same_case(vec![("Authorization", "a"), ("Authorization", "b")])]
#[case::mixed_case(vec![("Authorization", "a"), ("authorization", "b")])]
#[case::three(vec![
    ("Authorization", "a"),
    ("AUTHORIZATION", "b"),
    ("Authorization", "c"),
])]
fn rejects_duplicate_authorization_headers(#[case] pairs: Vec<(&str, &str)>) {
    let bytes: Vec<(&[u8], &[u8])> = pairs
        .iter()
        .map(|(name, value)| (name.as_bytes(), value.as_bytes()))
        .collect();
    assert_eq!(RawHeaders::from_pairs(&bytes), Err(DuplicateAuthorization));
}

#[test]
fn duplicate_rejection_carries_the_status_and_detail() {
    assert_eq!(DuplicateAuthorization::STATUS, StatusCode::BAD_REQUEST);
    assert_eq!(
        DuplicateAuthorization.to_string(),
        DUPLICATE_AUTHORIZATION_DETAIL
    );
}

#[test]
fn a_litellm_key_beside_authorization_is_not_a_duplicate() {
    let headers = parsed_headers(&[("Authorization", "a"), ("x-litellm-api-key", "k")]);
    assert_eq!(headers.lookup("authorization"), Some("a"));
}

#[test]
fn duplicate_other_headers_are_kept() {
    let headers = parsed_headers(&[("x-dup", "one"), ("x-dup", "two")]);
    assert_eq!(headers.items(), [("x-dup", "two")]);
}

#[test]
fn exact_name_duplicates_collapse_first_position_last_value() {
    let headers = parsed_headers(&[("X-Test", "one"), ("x-test", "two"), ("X-Test", "three")]);
    assert_eq!(headers.items(), [("x-test", "three"), ("x-test", "two")]);
}

#[test]
fn lookup_is_case_insensitive_and_returns_the_first_match() {
    let headers = parsed_headers(&[("X-Test", "one"), ("x-test", "two")]);
    assert_eq!(headers.lookup("x-TEST"), Some("one"));
    assert_eq!(headers.lookup("x-test"), Some("one"));
    assert_eq!(headers.lookup("absent"), None);
}

#[test]
fn names_and_values_decode_as_latin1() {
    let bytes: [(&[u8], &[u8]); 1] = [(b"X-Name", &[0xe9])];
    let headers = RawHeaders::from_pairs(&bytes).unwrap();
    assert_eq!(headers.lookup("x-name"), Some("\u{e9}"));
}

#[test]
fn empty_headers_hold_nothing() {
    let headers = RawHeaders::from_pairs(&[]).unwrap();
    assert!(headers.items().is_empty());
    assert_eq!(headers.lookup("authorization"), None);
}

#[test]
fn debug_output_lists_the_lowercased_names_and_no_values() {
    let headers = parsed_headers(&[("X-Api-Key", "sk-secret"), ("Accept", "text/plain")]);
    assert_eq!(
        format!("{headers:?}"),
        "RawHeaders { names: [\"x-api-key\", \"accept\"] }"
    );
}
