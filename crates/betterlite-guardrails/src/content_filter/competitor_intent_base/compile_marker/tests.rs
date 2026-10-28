#![cfg(test)]

use super::compile_marker;
use rstest::rstest;

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[case::blank(Some(" \t"))]
#[case::invalid(Some("["))]
#[case::unsupported(Some("(?=acme)"))]
fn absent_marker(#[case] pattern: Option<&str>) {
    assert!(compile_marker(pattern).is_none());
}

#[test]
fn case_insensitive_marker() {
    let marker = compile_marker(Some("compare")).unwrap();
    assert!(marker.is_match("COMPARE"));
}
