#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::error::FingerprintError;
use super::{stream_item_field, stream_item_fingerprint, stream_item_items};
use crate::guardrail_message_scope::StreamItem;

fn item(source: &str) -> StreamItem {
    StreamItem::Json(serde_json::from_str(source).unwrap())
}

#[rstest]
#[case::present(r#"{"usage":{"a":1}}"#, "usage", Some(r#"{"a":1}"#))]
#[case::missing(r#"{"usage":{"a":1}}"#, "type", None)]
#[case::null_counts_as_missing(r#"{"usage":null}"#, "usage", None)]
#[case::falsy_values_are_kept(r#"{"usage":0}"#, "usage", Some("0"))]
#[case::empty_string_is_kept(r#"{"usage":""}"#, "usage", Some(r#""""#))]
#[case::not_an_object(r#""text""#, "usage", None)]
#[case::array_item("[1]", "usage", None)]
fn a_field_comes_from_an_object_item(
    #[case] source: &str,
    #[case] field: &str,
    #[case] expected: Option<&str>,
) {
    let item = item(source);
    let found = stream_item_field(&item, field).map(|value| serde_json::to_string(value).unwrap());
    assert_eq!(found.as_deref(), expected);
}

#[test]
fn bytes_have_no_fields() {
    assert!(stream_item_field(&StreamItem::Bytes(b"{}".to_vec()), "a").is_none());
}

#[rstest]
#[case::array(r#"{"tools":[1,"a",null]}"#, 3)]
#[case::empty_array(r#"{"tools":[]}"#, 0)]
#[case::missing("{}", 0)]
#[case::null(r#"{"tools":null}"#, 0)]
#[case::object_is_not_a_sequence(r#"{"tools":{"a":1}}"#, 0)]
#[case::string_is_not_a_sequence(r#"{"tools":"abc"}"#, 0)]
#[case::item_is_not_an_object("[1,2]", 0)]
fn items_come_from_a_sequence_field(#[case] source: &str, #[case] expected: usize) {
    assert_eq!(stream_item_items(&item(source), "tools").len(), expected);
}

#[test]
fn items_keep_their_order() {
    let item = item(r#"{"tools":["b","a"]}"#);
    assert_eq!(
        stream_item_items(&item, "tools"),
        [
            GuardrailValue::Text("b".to_owned()),
            GuardrailValue::Text("a".to_owned())
        ]
    );
}

#[rstest]
#[case::field_order_does_not_matter(r#"{"b":1,"a":2}"#, r#"{"a":2,"b":1}"#)]
#[case::nested_field_order_does_not_matter(
    r#"{"x":{"b":[{"d":1,"c":2}],"a":null}}"#,
    r#"{"x":{"a":null,"b":[{"c":2,"d":1}]}}"#
)]
fn equal_items_have_the_same_fingerprint(#[case] left: &str, #[case] right: &str) {
    assert_eq!(
        stream_item_fingerprint(&item(left)).unwrap(),
        stream_item_fingerprint(&item(right)).unwrap()
    );
}

#[rstest]
#[case::different_value(r#"{"a":1}"#, r#"{"a":2}"#)]
#[case::different_key(r#"{"a":1}"#, r#"{"b":1}"#)]
#[case::list_order_matters(r#"{"a":[1,2]}"#, r#"{"a":[2,1]}"#)]
#[case::string_against_number(r#"{"a":"1"}"#, r#"{"a":1}"#)]
#[case::null_against_missing(r#"{"a":null}"#, "{}")]
fn different_items_have_different_fingerprints(#[case] left: &str, #[case] right: &str) {
    assert_ne!(
        stream_item_fingerprint(&item(left)).unwrap(),
        stream_item_fingerprint(&item(right)).unwrap()
    );
}

#[rstest]
#[case::sorted_keys(
    r#"{"b":[1,"x",null,true],"a":{"d":1.5,"c":"e"}}"#,
    r#"{"a":{"c":"e","d":1.5},"b":[1,"x",null,true]}"#
)]
#[case::text(r#""abc""#, r#""abc""#)]
#[case::empty_object("{}", "{}")]
fn the_fingerprint_is_sorted_compact_json(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(stream_item_fingerprint(&item(source)).unwrap(), expected);
}

#[test]
fn bytes_cannot_be_fingerprinted() {
    let result = stream_item_fingerprint(&StreamItem::Bytes(b"x".to_vec()));
    assert!(matches!(result, Err(FingerprintError::Bytes)));
}
