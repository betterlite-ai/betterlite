#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::Base64Error;
use super::Base64Truncator;

fn messages(encoded: &str) -> Json {
    serde_json::from_str(encoded).expect("messages decode")
}

fn truncator(max_payload_length: usize) -> Base64Truncator {
    Base64Truncator::new(max_payload_length).expect("pattern compiles")
}

fn payload(length: usize) -> String {
    "A".repeat(length)
}

fn data_uri(length: usize) -> String {
    format!("data:image/png;base64,{}", payload(length))
}

fn nested(levels: usize, leaf: Json) -> Json {
    let mut value = leaf;
    for _level in 0..levels {
        value = Json::Object(IndexMap::from([(String::from("next"), value)]));
    }
    value
}

fn text_of(value: &Json) -> &str {
    match value {
        Json::Text(text) => text,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("the value is not text")
        }
    }
}

#[test]
fn a_disabled_truncator_returns_the_messages_unchanged() {
    let original = messages(&format!(r#"{{"content": "{}"}}"#, data_uri(200)));
    let truncated = truncator(0)
        .truncate_messages(&original)
        .expect("truncated");
    assert_eq!(truncated, original);
}

#[test]
fn a_payload_at_the_limit_survives() {
    let uri = data_uri(64);
    let original = messages(&format!(r#"{{"content": "{uri}"}}"#));
    let truncated = truncator(64)
        .truncate_messages(&original)
        .expect("truncated");
    assert_eq!(truncated, original);
}

#[rstest]
#[case::kilobytes(2000, "data:image/png;base64,[base64_data truncated: 1.5KB]")]
#[case::bytes(65, "data:image/png;base64,[base64_data truncated: 48B]")]
#[case::megabytes(1_500_000, "data:image/png;base64,[base64_data truncated: 1.07MB]")]
fn logging_metadata_behavior_tests_behavior(#[case] length: usize, #[case] expected: &str) {
    let original = messages(&format!(r#"{{"content": "{}"}}"#, data_uri(length)));
    let truncated = truncator(64)
        .truncate_messages(&original)
        .expect("truncated");
    let Json::Object(fields) = truncated else {
        panic!("the result is an object");
    };
    assert_eq!(fields.get("content").map(text_of), Some(expected));
}

#[test]
fn every_data_uri_in_one_string_is_truncated() {
    let first = data_uri(200);
    let second = data_uri(300);
    let original = Json::Text(format!("a {first} b {second} c"));
    let truncated = truncator(64)
        .truncate_messages(&original)
        .expect("truncated");
    assert_eq!(
        text_of(&truncated),
        "a data:image/png;base64,[base64_data truncated: 150B] b data:image/png;base64,[base64_data truncated: 225B] c"
    );
}

#[test]
fn ordinary_text_and_other_scalars_survive() {
    let original = messages(
        r#"{"text": "no data here", "number": 7, "flag": true, "nothing": null, "list": ["data:text/plain;base64,AAAA"]}"#,
    );
    let truncated = truncator(4)
        .truncate_messages(&original)
        .expect("truncated");
    assert_eq!(truncated, original);
}

#[test]
fn nested_lists_and_objects_keep_their_shape() {
    let original = messages(&format!(
        r#"{{"messages": [{{"content": [{{"text": "{}"}}]}}]}}"#,
        data_uri(200)
    ));
    let truncated = truncator(64)
        .truncate_messages(&original)
        .expect("truncated");
    let encoded = serde_json::to_string(&truncated).expect("encodes");
    assert!(encoded.contains("base64_data truncated: 150B"));
    assert!(encoded.contains(r#""content""#));
    assert!(encoded.contains(r#""text""#));
}

#[test]
fn a_payload_below_the_depth_bound_is_truncated() {
    let original = nested(21, Json::Text(data_uri(2000)));
    let truncated = truncator(64)
        .truncate_messages(&original)
        .expect("truncated");
    let encoded = serde_json::to_string(&truncated).expect("encodes");
    assert!(encoded.contains("base64_data truncated: 1.5KB"));
}

#[test]
fn a_payload_below_the_depth_bound_keeps_its_text() {
    let original = nested(22, Json::Text(data_uri(2000)));
    let truncated = truncator(64)
        .truncate_messages(&original)
        .expect("truncated");
    let encoded = serde_json::to_string(&truncated).expect("encodes");
    assert!(encoded.contains(&payload(2000)));
    assert!(!encoded.contains("truncated"));
}

#[test]
fn the_input_is_not_modified() {
    let original = messages(&format!(r#"{{"content": "{}"}}"#, data_uri(200)));
    let before = serde_json::to_string(&original).expect("encodes");
    let _truncated = truncator(64)
        .truncate_messages(&original)
        .expect("truncated");
    assert_eq!(serde_json::to_string(&original).expect("encodes"), before);
}

#[rstest]
#[case::unset(None, 65, true)]
#[case::unset_at_the_default(None, 64, false)]
#[case::empty(Some(""), 65, true)]
#[case::raised(Some("1000"), 999, false)]
#[case::raised_and_exceeded(Some("1000"), 1001, true)]
#[case::zero(Some("0"), 200, false)]
#[case::negative(Some("-1"), 200, false)]
fn the_limit_comes_from_the_environment_and_defaults_to_64(
    #[case] configured: Option<&str>,
    #[case] length: usize,
    #[case] truncated: bool,
) {
    let original = messages(&format!(r#"{{"content": "{}"}}"#, data_uri(length)));
    let result = Base64Truncator::from_env(configured)
        .expect("limit parses")
        .truncate_messages(&original)
        .expect("truncated");
    assert_eq!(result != original, truncated);
}

#[test]
fn a_limit_that_is_not_a_number_is_refused() {
    let Err(error) = Base64Truncator::from_env(Some("lots")) else {
        panic!("the limit is refused");
    };
    assert!(matches!(error, Base64Error::Env(_)));
}
