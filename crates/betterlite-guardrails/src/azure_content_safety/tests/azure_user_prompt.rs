use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::azure_content_safety::error::AzureContentSafetyError;
use crate::azure_content_safety::user_prompt::get_user_prompt_from_request;

fn data(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

#[test]
fn returns_the_last_consecutive_user_run() {
    let request = data(
        r#"{"messages":[
            {"role":"user","content":"first"},
            {"role":"assistant","content":"reply"},
            {"role":"user","content":"second"},
            {"role":"user","content":"third"}
        ]}"#,
    );
    assert_eq!(
        get_user_prompt_from_request(&request, "completion").unwrap(),
        Some("second\nthird".to_owned())
    );
}

#[test]
fn returns_none_when_the_last_message_is_not_from_the_user() {
    let request = data(
        r#"{"messages":[{"role":"user","content":"first"},{"role":"assistant","content":"reply"}]}"#,
    );
    assert_eq!(
        get_user_prompt_from_request(&request, "completion").unwrap(),
        None
    );
}

#[test]
fn returns_none_without_messages() {
    assert_eq!(
        get_user_prompt_from_request(&data("{}"), "completion").unwrap(),
        None
    );
}

#[test]
fn returns_none_for_null_messages() {
    assert_eq!(
        get_user_prompt_from_request(&data(r#"{"messages":null}"#), "completion").unwrap(),
        None
    );
}

#[test]
fn joins_the_text_parts_of_the_user_content() {
    let request = data(
        r#"{"messages":[{"role":"user","content":[
            {"type":"text","text":"hello "},
            {"type":"image_url","image_url":{"url":"https://example/image.png"}},
            {"type":"text","text":"world"}
        ]}]}"#,
    );
    assert_eq!(
        get_user_prompt_from_request(&request, "acompletion").unwrap(),
        Some("hello world".to_owned())
    );
}

#[test]
fn trims_the_user_prompt() {
    let request = data(r#"{"messages":[{"role":"user","content":"  hello  "}]}"#);
    assert_eq!(
        get_user_prompt_from_request(&request, "completion").unwrap(),
        Some("hello".to_owned())
    );
}

#[test]
fn returns_none_for_a_blank_user_prompt() {
    let request = data(r#"{"messages":[{"role":"user","content":"   "}]}"#);
    assert_eq!(
        get_user_prompt_from_request(&request, "completion").unwrap(),
        None
    );
}

#[test]
fn returns_none_for_a_null_content() {
    let request = data(r#"{"messages":[{"role":"user","content":null}]}"#);
    assert_eq!(
        get_user_prompt_from_request(&request, "completion").unwrap(),
        None
    );
}

#[test]
fn reads_a_responses_text_input() {
    let request = data(r#"{"input":"hello"}"#);
    assert_eq!(
        get_user_prompt_from_request(&request, "aresponses").unwrap(),
        Some("hello".to_owned())
    );
    assert_eq!(
        get_user_prompt_from_request(&request, "responses").unwrap(),
        Some("hello".to_owned())
    );
}

#[test]
fn reads_the_last_user_run_of_responses_items() {
    let request = data(
        r#"{"input":[
            {"role":"user","content":"first"},
            {"role":"assistant","content":"reply"},
            {"role":"user","content":"second"}
        ]}"#,
    );
    assert_eq!(
        get_user_prompt_from_request(&request, "aresponses").unwrap(),
        Some("second".to_owned())
    );
}

#[test]
fn skips_responses_items_without_a_role() {
    let request = data(
        r#"{"input":[
            {"content":"ignored"},
            "bare string",
            {"role":"user","content":"kept"}
        ]}"#,
    );
    assert_eq!(
        get_user_prompt_from_request(&request, "aresponses").unwrap(),
        Some("kept".to_owned())
    );
}

#[test]
fn reads_content_parts_of_responses_items() {
    let request =
        data(r#"{"input":[{"role":"user","content":[{"type":"input_text","text":"hello"}]}]}"#);
    assert_eq!(
        get_user_prompt_from_request(&request, "aresponses").unwrap(),
        Some("hello".to_owned())
    );
}

#[test]
fn returns_none_for_responses_input_of_another_shape() {
    assert_eq!(
        get_user_prompt_from_request(&data(r#"{"input":5}"#), "aresponses").unwrap(),
        None
    );
    assert_eq!(
        get_user_prompt_from_request(&data(r#"{"input":null}"#), "aresponses").unwrap(),
        None
    );
    assert_eq!(
        get_user_prompt_from_request(&data(r#"{"input":{"role":"user"}}"#), "aresponses").unwrap(),
        None
    );
}

#[test]
fn uses_messages_for_a_completion_call_type() {
    let request =
        data(r#"{"input":"from input","messages":[{"role":"user","content":"from messages"}]}"#);
    assert_eq!(
        get_user_prompt_from_request(&request, "completion").unwrap(),
        Some("from messages".to_owned())
    );
}

#[test]
fn rejects_messages_that_are_not_a_list() {
    let error =
        get_user_prompt_from_request(&data(r#"{"messages":"hi"}"#), "completion").unwrap_err();
    assert!(matches!(
        error,
        AzureContentSafetyError::MessagesShape { .. }
    ));
}

#[test]
fn rejects_a_message_that_is_not_an_object() {
    let error =
        get_user_prompt_from_request(&data(r#"{"messages":["hi"]}"#), "completion").unwrap_err();
    assert!(matches!(error, AzureContentSafetyError::MessageJson { .. }));
}
