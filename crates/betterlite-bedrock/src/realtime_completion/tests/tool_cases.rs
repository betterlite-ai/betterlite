use rstest::rstest;

use super::support::{EVENT_1, FixedIds, open_state, wire};
use crate::realtime_completion::error::CompletionError;
use crate::realtime_completion::records::{DeltaType, ResponseState};
use crate::realtime_completion::tool_input::{ToolUseEvent, decode_realtime_tool_input};
use crate::realtime_completion::tool_use::tool_use_events;

pub(super) fn tool_use_event() {
    let state = open_state(Some(DeltaType::Text), None);
    let event: ToolUseEvent = serde_json::from_str(
        r#"{"toolUse":{"toolUseId":"call_1","toolName":"get_weather","input":"{\"city\":\"Paris\"}"}}"#,
    )
    .expect("a toolUse fixture");
    let output =
        tool_use_events(&state, &event, &FixedIds::sequence(1)).expect("the tool call completes");
    assert_eq!(output.tool_call_id, "call_1");
    assert_eq!(output.tool_name, "get_weather");
    assert_eq!(
        wire(&output.events),
        [format!(
            r#"{{"type":"response.function_call_arguments.done","event_id":"{EVENT_1}","response_id":"resp_1","item_id":"item_1","output_index":0,"call_id":"call_1","name":"get_weather","arguments":"{{\"city\":\"Paris\"}}"}}"#
        )]
    );
    assert_eq!(state, open_state(Some(DeltaType::Text), None));
}

#[rstest]
#[case::without_item(None, Some("resp_1"))]
#[case::without_response(Some("item_1"), None)]
#[case::without_either(None, None)]
fn tool_use_without_ids_emits_nothing(#[case] item: Option<&str>, #[case] response: Option<&str>) {
    let state = ResponseState {
        output_item_id: item.map(str::to_owned),
        response_id: response.map(str::to_owned),
        conversation_id: Some("conv_1".to_owned()),
        delta_type: None,
        delta_chunks: None,
    };
    let event: ToolUseEvent =
        serde_json::from_str(r#"{"toolUse":{"toolUseId":"call_1","toolName":"get_weather"}}"#)
            .expect("a toolUse fixture");
    let output =
        tool_use_events(&state, &event, &FixedIds::sequence(1)).expect("the guard returns");
    assert!(output.events.is_empty());
    assert_eq!(output.tool_call_id, "");
    assert_eq!(output.tool_name, "");
}

#[rstest]
#[case::absent(r#"{"toolUse":{}}"#, "{}")]
#[case::object(r#"{"toolUse":{"input":{"a":1}}}"#, r#"{"a":1}"#)]
#[case::json_text(r#"{"toolUse":{"input":"{\"a\":1}"}}"#, r#"{"a":1}"#)]
#[case::text_with_spaces(r#"{"toolUse":{"input":" {\"a\": 1} "}}"#, r#"{"a": 1}"#)]
#[case::text_not_json(r#"{"toolUse":{"input":"not json"}}"#, "{}")]
#[case::empty_text(r#"{"toolUse":{"input":""}}"#, "{}")]
#[case::nan_text(r#"{"toolUse":{"input":"NaN"}}"#, "{}")]
#[case::null(r#"{"toolUse":{"input":null}}"#, "null")]
#[case::null_text(r#"{"toolUse":{"input":"null"}}"#, "null")]
#[case::array(r#"{"toolUse":{"input":[1,2]}}"#, "[1,2]")]
#[case::array_text(r#"{"toolUse":{"input":"[1]"}}"#, "[1]")]
#[case::number(r#"{"toolUse":{"input":42}}"#, "42")]
#[case::number_text(r#"{"toolUse":{"input":"5"}}"#, "5")]
#[case::boolean(r#"{"toolUse":{"input":true}}"#, "true")]
fn tool_use_arguments(#[case] text: &str, #[case] arguments: &str) {
    let state = open_state(None, None);
    let event: ToolUseEvent = serde_json::from_str(text).expect("a toolUse fixture");
    let output =
        tool_use_events(&state, &event, &FixedIds::sequence(1)).expect("the tool call completes");
    let wired = wire(&output.events);
    let escaped = serde_json::to_string(arguments).expect("text serializes");
    assert!(
        wired[0].ends_with(&format!(r#","arguments":{escaped}}}"#)),
        "{}",
        wired[0]
    );
}

#[rstest]
#[case::absent(r#"{"toolUse":{}}"#)]
#[case::null(r#"{"toolUse":{"toolUseId":null,"toolName":null}}"#)]
fn tool_use_ids_default_to_empty_text(#[case] text: &str) {
    let state = open_state(None, None);
    let event: ToolUseEvent = serde_json::from_str(text).expect("a toolUse fixture");
    let output =
        tool_use_events(&state, &event, &FixedIds::sequence(1)).expect("the tool call completes");
    assert_eq!(output.tool_call_id, "");
    assert_eq!(output.tool_name, "");
    assert!(wire(&output.events)[0].contains(r#""call_id":"","name":"","#));
}

#[rstest]
#[case::missing_tool_use("{}")]
#[case::tool_use_text(r#"{"toolUse":"x"}"#)]
#[case::numeric_id(r#"{"toolUse":{"toolUseId":5}}"#)]
#[case::list_name(r#"{"toolUse":{"toolName":["a"]}}"#)]
fn tool_use_with_wrong_types_is_an_error(#[case] text: &str) {
    assert!(serde_json::from_str::<ToolUseEvent>(text).is_err());
}

#[rstest]
#[case::object(br#"{"a":1}"#.as_slice(), Some(r#"{"a":1}"#))]
#[case::json_text(br#""{\"a\":1}""#.as_slice(), Some(r#"{"a":1}"#))]
#[case::not_json_text(br#""nope""#.as_slice(), Some("{}"))]
#[case::big_number(b"123456789012345678901234567890".as_slice(), Some("123456789012345678901234567890"))]
#[case::empty(b"".as_slice(), None)]
#[case::truncated(br#"{"a":"#.as_slice(), None)]
#[case::trailing_text(b"{} x".as_slice(), None)]
fn decode_realtime_tool_input_cases(#[case] bytes: &[u8], #[case] expected: Option<&str>) {
    let decoded = decode_realtime_tool_input(bytes);
    match expected {
        Some(text) => assert_eq!(decoded.expect("the input decodes").get(), text),
        None => assert!(matches!(decoded, Err(CompletionError::ToolInput { .. }))),
    }
}
