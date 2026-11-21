#![cfg(test)]

use rstest::rstest;

use super::body_text;

pub(crate) mod support;

#[test]
fn reads_a_text_member_of_the_body() {
    assert_eq!(
        body_text(&support::data(r#"{"model": "gpt-4"}"#), "model"),
        Some("gpt-4")
    );
}

#[test]
fn a_missing_member_reads_as_absent() {
    assert_eq!(
        body_text(&support::data(r#"{"user": "u1"}"#), "model"),
        None
    );
}

#[rstest]
#[case::null(r#"{"model": null}"#)]
#[case::boolean(r#"{"model": true}"#)]
#[case::number(r#"{"model": 5}"#)]
#[case::list(r#"{"model": ["gpt-4"]}"#)]
#[case::object(r#"{"model": {"name": "gpt-4"}}"#)]
fn a_non_text_member_reads_as_absent(#[case] wire: &str) {
    assert_eq!(body_text(&support::data(wire), "model"), None);
}

#[test]
fn an_empty_text_member_reads_as_the_empty_text() {
    assert_eq!(
        body_text(&support::data(r#"{"model": ""}"#), "model"),
        Some("")
    );
}
