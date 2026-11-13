#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::build_stream_error_items;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, object};

#[rstest]
#[case::plain(
    json(r#"{"message": "blocked", "type": "guardrail_violation"}"#),
    "data: {\"error\":{\"message\":\"blocked\",\"type\":\"guardrail_violation\"}}\n\n"
)]
#[case::nested_and_null(
    json(r#"{"code": null, "message": "m", "param": {"a": [1, "b"]}}"#),
    "data: {\"error\":{\"code\":null,\"message\":\"m\",\"param\":{\"a\":[1,\"b\"]}}}\n\n"
)]
#[case::special_characters_are_escaped(
    json(r#"{"message": "say \"hi\"\nbye"}"#),
    "data: {\"error\":{\"message\":\"say \\\"hi\\\"\\nbye\"}}\n\n"
)]
#[case::empty_error(json("{}"), "data: {\"error\":{}}\n\n")]
fn the_error_goes_out_as_one_data_frame(#[case] error: JsonValue, #[case] expected: &str) {
    let items = build_stream_error_items(&object(error)).unwrap();
    assert_eq!(items, [expected.as_bytes().to_vec()]);
}
