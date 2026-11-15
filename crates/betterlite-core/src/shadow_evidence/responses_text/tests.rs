#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::final_text;

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn response(output: &str) -> String {
    format!(r#"{{"id":"resp_1","created_at":1,"output":{output}}}"#)
}

fn with_message(content: &str) -> String {
    format!(
        r#"{{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":{content}}}"#
    )
}

const MESSAGE: &str = r#"{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello ","annotations":[]},{"type":"output_text","text":"world","annotations":[]}]}"#;

#[rstest]
#[case::message_text(
    &response(MESSAGE),
    "Hello world"
)]
#[case::two_messages(
    &response(&format!(
        r#"[{MESSAGE},{{"type":"message","id":"msg_2","role":"assistant","status":"completed","content":[{{"type":"output_text","text":"!","annotations":[]}}]}}]"#
    )),
    "Hello world!"
)]
#[case::function_call(
    &response(r#"[{"type":"function_call","id":"fc_1","call_id":"c1","name":"search","arguments":"{\"q\":\"x\"}","status":"completed"}]"#),
    r#"[tool call] search({"q":"x"})"#
)]
#[case::custom_tool_call(
    &response(r#"[{"type":"custom_tool_call","id":"cc_1","call_id":"c2","name":"apply","input":"raw"}]"#),
    "[tool call] apply(raw)"
)]
#[case::prose_and_both_calls(
    &response(&format!(
        "[{},{},{}]",
        with_message(r#"[{"type":"output_text","text":"Checking.","annotations":[]}]"#),
        r#"{"type":"function_call","call_id":"c1","name":"search","arguments":"{}"}"#,
        r#"{"type":"custom_tool_call","call_id":"c2","name":"apply","input":"raw"}"#
    )),
    "Checking. [tool call] search({}), [tool call] apply(raw)"
)]
#[case::call_without_text(
    &response(r#"[{"type":"function_call","call_id":"c1","name":"search","arguments":"{}"}]"#),
    "[tool call] search({})"
)]
#[case::empty_output(&response("[]"), "")]
#[case::missing_id(
    r#"{"created_at":1,"output":[{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello","annotations":[]}]}]}"#,
    ""
)]
#[case::missing_created_at(
    &format!(r#"{{"id":"resp_1","output":[{MESSAGE}]}}"#),
    ""
)]
#[case::output_not_a_list(&response(r#""x""#), "")]
#[case::output_item_not_an_object(&response(r#"["text"]"#), "")]
#[case::non_object_response("5", "")]
#[case::null_response("null", "")]
#[case::refusal_part_is_ignored(
    &response(&format!(
        "[{}]",
        with_message(
            r#"[{"type":"output_text","text":"ok","annotations":[]},{"type":"refusal","refusal":"no"}]"#
        )
    )),
    "ok"
)]
#[case::unknown_item_adds_nothing(
    &response(&format!(
        r#"[{MESSAGE},{{"type":"image_generation_call","id":"img_1"}}]"#
    )),
    "Hello world"
)]
#[case::partially_typed_call_keeps_its_invocation(
    &response(r#"[{"type":"function_call","name":"search","arguments":"{}"}]"#),
    "[tool call] search({})"
)]
#[case::message_without_status_still_reads_text(
    &response(r#"[{"type":"message","id":"msg_1","role":"assistant","content":[{"type":"output_text","text":"Hello","annotations":[]}]}]"#),
    "Hello"
)]
#[case::prose_only_keeps_its_whitespace(
    &response(&format!(
        "[{}]",
        with_message(r#"[{"type":"output_text","text":" hi ","annotations":[]}]"#)
    )),
    " hi "
)]
#[case::trailing_prose_space_is_trimmed_around_a_call(
    &response(&format!(
        "[{},{}]",
        with_message(r#"[{"type":"output_text","text":"hi ","annotations":[]}]"#),
        r#"{"type":"function_call","call_id":"c1","name":"f","arguments":"{}"}"#
    )),
    "hi  [tool call] f({})"
)]
#[case::empty_message_text_adds_nothing(
    &response(&format!(
        "[{}]",
        with_message(r#"[{"type":"output_text","text":"","annotations":[]}]"#)
    )),
    ""
)]
#[case::a_partial_type_is_not_a_call(
    &response(r#"[{"type":"function_call_extra","call_id":"c1","name":"search","arguments":"{}"}]"#),
    ""
)]
fn final_text_reads_messages_and_calls(#[case] response: &str, #[case] expected: &str) {
    assert_eq!(final_text(&json(response)), expected);
}
