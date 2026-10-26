#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::{
    RoutingValue, apply_quality_headers, complexity_headers, decode_routing_header_value,
    replace_complexity_headers,
};

fn value(text: &str) -> RoutingValue {
    serde_json::from_str(text).unwrap()
}

#[test]
fn complexity_projects_exact_fields() {
    let request = value(
        r#"{"metadata":{"routing_decision":{"router_type":"complexity","tier":" high ","cause":" score ","score":0.75,"tier_litellm_params":{"reasoning_effort":" high "}}}}"#,
    );
    assert_eq!(
        complexity_headers(&request),
        BTreeMap::from([
            (
                "x-litellm-complexity-router-cause".to_owned(),
                "score".to_owned()
            ),
            (
                "x-litellm-complexity-router-reasoning-effort".to_owned(),
                "high".to_owned()
            ),
            (
                "x-litellm-complexity-router-score".to_owned(),
                "0.75".to_owned()
            ),
            (
                "x-litellm-complexity-router-tier".to_owned(),
                "high".to_owned()
            ),
        ])
    );
}

#[rstest]
#[case::null("null")]
#[case::array("[]")]
#[case::wrong_type("\"complexity\"")]
fn complexity_rejects_non_mappings(#[case] text: &str) {
    assert!(complexity_headers(&value(text)).is_empty());
}

#[test]
fn complexity_null_litellm_metadata_wins() {
    let request = value(
        r#"{"litellm_metadata":null,"metadata":{"routing_decision":{"router_type":"complexity","tier":"high"}}}"#,
    );
    assert!(complexity_headers(&request).is_empty());
}

#[test]
fn replacement_removes_stale_prefix_and_keeps_other_values() {
    let existing = BTreeMap::from([
        (
            "x-litellm-complexity-router-tier".to_owned(),
            value("\"old\""),
        ),
        (
            "X-LiteLLM-complexity-router-cause".to_owned(),
            value("\"keep\""),
        ),
        ("unrelated".to_owned(), value("false")),
    ]);
    let new = BTreeMap::from([
        (
            "x-litellm-complexity-router-score".to_owned(),
            "1".to_owned(),
        ),
        ("unrelated".to_owned(), "new".to_owned()),
    ]);
    assert_eq!(
        replace_complexity_headers(&existing, &new),
        BTreeMap::from([
            (
                "X-LiteLLM-complexity-router-cause".to_owned(),
                value("\"keep\"")
            ),
            ("unrelated".to_owned(), value("\"new\"")),
            (
                "x-litellm-complexity-router-score".to_owned(),
                value("\"1\"")
            ),
        ])
    );
}

#[test]
fn quality_uses_metadata_and_retains_false_zero_empty() {
    let request = value(
        r#"{"litellm_metadata":{"quality_router_decision":{"routed_model":"wrong"}},"metadata":{"quality_router_decision":{"routed_model":"model","quality_tier":"","routed_via":false,"matched_keyword":0,"complexity_tier":null}}}"#,
    );
    let mut headers = BTreeMap::new();
    apply_quality_headers(&mut headers, &request);
    assert_eq!(
        headers,
        BTreeMap::from([
            (
                "x-litellm-quality-router-keyword".to_owned(),
                value("\"0\"")
            ),
            (
                "x-litellm-quality-router-model".to_owned(),
                value("\"model\"")
            ),
            ("x-litellm-quality-router-tier".to_owned(), value("\"\"")),
            (
                "x-litellm-quality-router-via".to_owned(),
                value("\"False\"")
            ),
        ])
    );
}

#[rstest]
#[case::integer("3", "3")]
#[case::integral_float("1.0", "1.0")]
#[case::large_fixed("1e15", "1000000000000000.0")]
#[case::large_exponent("1e16", "1e+16")]
#[case::small_fixed("0.0001", "0.0001")]
#[case::small_exponent("1.5e-5", "1.5e-05")]
#[case::long_exponent("123456789012345680.0", "1.2345678901234568e+17")]
#[case::negative_zero("-0.0", "-0.0")]
fn complexity_score_prints_like_python(#[case] score: &str, #[case] expected: &str) {
    let request = value(&format!(
        r#"{{"metadata":{{"routing_decision":{{"router_type":"complexity","score":{score}}}}}}}"#
    ));
    assert_eq!(
        complexity_headers(&request),
        BTreeMap::from([(
            "x-litellm-complexity-router-score".to_owned(),
            expected.to_owned()
        )])
    );
}

#[rstest]
#[case::float("2.5e-7", "2.5e-07")]
#[case::list(
    r#"["a", 1, true, null, 1.0, {"k": "it's"}]"#,
    r#"['a', 1, True, None, 1.0, {'k': "it's"}]"#
)]
#[case::insertion_order(r#"{"b": [], "a": {}}"#, "{'b': [], 'a': {}}")]
#[case::both_quotes(r#"["'\""]"#, r#"['\'"']"#)]
#[case::escapes(
    concat!(r#"["a\nb\\\t\r\u0001\u007f\u00a0\u2028\u200b\u0378\udb80\udc00 "#, "\u{e9}", r#""]"#),
    concat!(r"['a\nb\\\t\r\x01\x7f\xa0\u2028\u200b\u0378\U000f0000 ", "\u{e9}", r"']")
)]
fn quality_values_print_like_python(#[case] field: &str, #[case] expected: &str) {
    let request = value(&format!(
        r#"{{"metadata":{{"quality_router_decision":{{"routed_model":{field}}}}}}}"#
    ));
    let mut headers = BTreeMap::new();
    apply_quality_headers(&mut headers, &request);
    assert_eq!(
        headers,
        BTreeMap::from([(
            "x-litellm-quality-router-model".to_owned(),
            RoutingValue::String(expected.to_owned())
        )])
    );
}

#[rstest]
#[case::trim("  value  ", Some("value"))]
#[case::unicode("\u{e9}", None)]
#[case::newline("a\nb", None)]
#[case::space_boundary(" ", None)]
#[case::tilde("~", Some("~"))]
fn routing_header_value_boundaries(#[case] value: &str, #[case] expected: Option<&str>) {
    assert_eq!(decode_routing_header_value(value).as_deref(), expected);
}

#[rstest]
fn replays_the_routing_header_value_corpus(
    #[files("fuzz/corpus/routing_header_value/*")] path: PathBuf,
) {
    let text = fs::read_to_string(path).unwrap();
    drop(decode_routing_header_value(&text));
}
