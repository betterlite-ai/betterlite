#![cfg(test)]

use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_input::{ResponsesInput, ResponsesItem};
use rstest::rstest;
use serde::de::DeserializeOwned;

use super::{create_follow_up_input, is_persistence_disabled};
use crate::api::responses_create::call::ResponsesCreateCall;
use crate::api::responses_mcp_results::McpToolResult;

fn parse<T: DeserializeOwned>(document: &str) -> T {
    serde_json::from_str(document).expect("the fixture decodes")
}

fn response_with(output: &str) -> ResponsesApiResponse {
    let body = format!(r#"{{"id":"resp_1","created_at":1,"output":{output}}}"#);
    parse(&body)
}

#[rstest]
#[case::absent(r#"{"model":"gpt-4","input":"hello"}"#, false)]
#[case::null(r#"{"model":"gpt-4","input":"hello","store":null}"#, false)]
#[case::store_false(r#"{"model":"gpt-4","input":"hello","store":false}"#, true)]
#[case::store_true(r#"{"model":"gpt-4","input":"hello","store":true}"#, false)]
fn store_false_only(#[case] request: &str, #[case] expected: bool) {
    let call: ResponsesCreateCall = parse(request);
    assert_eq!(is_persistence_disabled(&call), expected);
}

#[rstest]
#[case::text(
    Some(r#""hello""#),
    r#"[{"type":"message","role":"user","content":"hello"}]"#
)]
#[case::items(
    Some(
        r#"[{"type":"message","role":"user","content":"hi"},{"type":"function_call_output","call_id":"call_1","output":"42"}]"#
    ),
    r#"[{"type":"message","role":"user","content":"hi"},{"type":"function_call_output","call_id":"call_1","output":"42"}]"#
)]
#[case::empty_text(Some(r#""""#), r#"[]"#)]
#[case::empty_list(Some(r#"[]"#), r#"[]"#)]
#[case::none(None, r#"[]"#)]
fn original_input_first(#[case] original: Option<&str>, #[case] expected: &str) {
    let response = response_with("[]");
    let original_input = original.map(parse::<ResponsesInput>);
    let items = create_follow_up_input(&response, &[], original_input.as_ref(), false)
        .expect("the original input is copied");
    assert_eq!(items, parse::<Vec<ResponsesItem>>(expected));
}

const MIXED_OUTPUT: &str = r#"[
    {"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[{"type":"output_text","text":"one","annotations":[]}]},
    {"type":"function_call","call_id":"call_1","name":"weather","arguments":"{\"city\":\"Paris\"}"},
    {"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"thinking"}],"encrypted_content":"enc"},
    {"type":"image_generation_call","id":"ig_1","status":"completed","result":null},
    {"type":"message","id":"msg_2","status":"completed","role":"assistant","content":[{"type":"output_text","text":"two","annotations":[]}]},
    {"type":"reasoning","summary":[],"content":null},
    {"type":"message","id":"msg_3","status":"completed","role":"assistant"},
    {"type":"message","id":"msg_4","role":"assistant","content":"plain"},
    {"type":"message","id":"msg_5","role":"assistant","content":null}
]"#;

const MIXED_KEPT: &str = r#"[
    {"type":"message","role":"assistant","content":[{"type":"output_text","text":"one","annotations":[]},{"type":"output_text","text":"two","annotations":[]},"plain",null]},
    {"type":"function_call","call_id":"call_1","name":"weather","arguments":"{\"city\":\"Paris\"}"},
    {"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"thinking"}],"encrypted_content":"enc"},
    {"type":"reasoning","summary":[],"content":null}
]"#;

const MIXED_DROPPED: &str = r#"[
    {"type":"message","role":"assistant","content":[{"type":"output_text","text":"one","annotations":[]},{"type":"output_text","text":"two","annotations":[]},"plain",null]},
    {"type":"function_call","call_id":"call_1","name":"weather","arguments":"{\"city\":\"Paris\"}"}
]"#;

#[rstest]
#[case::reasoning_kept(true, MIXED_KEPT)]
#[case::reasoning_dropped(false, MIXED_DROPPED)]
fn message_content_before_turn_items(#[case] preserve_reasoning: bool, #[case] expected: &str) {
    let response = response_with(MIXED_OUTPUT);
    let items = create_follow_up_input(&response, &[], None, preserve_reasoning)
        .expect("the output items copy");
    assert_eq!(items, parse::<Vec<ResponsesItem>>(expected));
}

#[rstest]
#[case::call_id_before_id(
    r#"[{"type":"function_call","call_id":"call_1","id":"fc_1","name":"weather","arguments":"{}"}]"#,
    r#"[{"type":"function_call","call_id":"call_1","name":"weather","arguments":"{}"}]"#
)]
#[case::id_falls_back(
    r#"[{"type":"function_call","id":"fc_2","name":"weather","arguments":"{}"}]"#,
    r#"[{"type":"function_call","call_id":"fc_2","name":"weather","arguments":"{}"}]"#
)]
#[case::empty_call_id_falls_back(
    r#"[{"type":"function_call","call_id":"","id":"fc_3","name":"weather","arguments":"{}"}]"#,
    r#"[{"type":"function_call","call_id":"fc_3","name":"weather","arguments":"{}"}]"#
)]
#[case::no_call_id(
    r#"[{"type":"function_call","name":"weather","arguments":"{}"}]"#,
    r#"[]"#
)]
#[case::no_name(
    r#"[{"type":"function_call","call_id":"call_5","arguments":"{}"}]"#,
    r#"[]"#
)]
#[case::null_arguments(
    r#"[{"type":"function_call","call_id":"call_6","name":"weather","arguments":null}]"#,
    r#"[{"type":"function_call","call_id":"call_6","name":"weather","arguments":null}]"#
)]
#[case::missing_arguments(
    r#"[{"type":"function_call","call_id":"call_7","name":"weather"}]"#,
    r#"[{"type":"function_call","call_id":"call_7","name":"weather","arguments":null}]"#
)]
fn function_call_fields(#[case] output: &str, #[case] expected: &str) {
    let response = response_with(output);
    let items =
        create_follow_up_input(&response, &[], None, false).expect("the function call copies");
    assert_eq!(items, parse::<Vec<ResponsesItem>>(expected));
}

// a function call, then one function_call_output per result in result order, a null call id kept
#[test]
fn tool_outputs_in_order() {
    let response = response_with(
        r#"[{"type":"function_call","call_id":"call_1","name":"weather","arguments":"{}"}]"#,
    );
    let results: Vec<McpToolResult> = parse(
        r#"[{"tool_call_id":"call_1","result":"42","name":"weather"},{"tool_call_id":null,"result":"boom","name":null}]"#,
    );
    let items = create_follow_up_input(&response, &results, None, false).expect("the outputs copy");
    assert_eq!(
        items,
        parse::<Vec<ResponsesItem>>(
            r#"[{"type":"function_call","call_id":"call_1","name":"weather","arguments":"{}"},{"type":"function_call_output","call_id":"call_1","output":"42"},{"type":"function_call_output","call_id":null,"output":"boom"}]"#
        )
    );
}
