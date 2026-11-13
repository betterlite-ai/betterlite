#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::convert_tool_call_to_dict;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{
    guardrail, json, object,
};

#[rstest]
#[case::full_tool_call(json(
    r#"{"id": "1", "type": "function", "function": {"name": "f", "arguments": "{}"}}"#
))]
#[case::extra_fields_are_kept(json(r#"{"id": "1", "index": 0, "function": {"name": "f"}}"#))]
#[case::empty_object(json("{}"))]
fn a_tool_call_object_is_returned_as_the_dict_it_is(#[case] tool_call: JsonValue) {
    let converted = convert_tool_call_to_dict(&guardrail(tool_call.clone()));
    assert_eq!(converted, Some(object(tool_call)));
}

#[rstest]
#[case::null(json("null"))]
#[case::string(json(r#""call_1""#))]
#[case::number(json("1"))]
#[case::list(json(r#"[{"id": "1"}]"#))]
fn anything_else_is_invalid_and_dropped(#[case] tool_call: JsonValue) {
    assert_eq!(convert_tool_call_to_dict(&guardrail(tool_call)), None);
}
