use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::support::{messages, object};
use crate::langflow_chat::error::{LangFlowError, TWEAKS_MESSAGE};
use crate::langflow_chat::request::{RequestInput, transform_request};

fn input_value(wire: &str) -> String {
    let messages = messages(wire);
    transform_request(&RequestInput {
        messages: &messages,
        optional_params: &object("{}"),
    })
    .expect("the fixture request builds")
    .input_value
}

#[test]
fn the_last_user_message_becomes_the_input_value() {
    assert_eq!(
        input_value(
            r#"[
                {"role":"system","content":"be nice"},
                {"role":"user","content":"first"},
                {"role":"assistant","content":"ignored"},
                {"role":"user","content":"second"}
            ]"#
        ),
        "second"
    );
}

#[test]
fn a_user_message_wins_over_a_later_assistant_message() {
    assert_eq!(
        input_value(
            r#"[
                {"role":"user","content":"question"},
                {"role":"assistant","content":"answer"}
            ]"#
        ),
        "question"
    );
}

#[test]
fn without_a_user_message_the_last_message_is_used() {
    assert_eq!(
        input_value(
            r#"[
                {"role":"system","content":"be nice"},
                {"role":"assistant","content":"answer"}
            ]"#
        ),
        "answer"
    );
}

#[test]
fn without_messages_the_input_value_is_empty() {
    assert_eq!(input_value("[]"), "");
}

#[test]
fn list_content_goes_through_the_shared_flattener() {
    assert_eq!(
        input_value(
            r#"[
                {"role":"user","content":[
                    {"type":"text","text":"a"},
                    {"type":"image_url","image_url":{"url":"http://img"}},
                    {"type":"text","text":"b"}
                ]}
            ]"#
        ),
        "ab"
    );
}

#[test]
fn list_content_keeps_the_search_results_text() {
    assert_eq!(
        input_value(
            r#"[
                {"role":"user","content":[{"type":"text","text":"a"}],
                 "search_results":[{"source":"s","title":"t","content":[{"type":"text","text":"c"}],"citations":{"x":1}}]}
            ]"#
        ),
        r#"astc{"x":1}"#
    );
}

#[test]
fn text_content_leaves_search_results_out() {
    assert_eq!(
        input_value(
            r#"[
                {"role":"user","content":"a",
                 "search_results":[{"source":"s","title":"t","content":[{"type":"text","text":"c"}]}]}
            ]"#
        ),
        "a"
    );
}

#[test]
fn the_body_defaults_the_types_to_chat() {
    let optional_params = object("{}");
    let body = transform_request(&RequestInput {
        messages: &messages(r#"[{"role":"user","content":"hi"}]"#),
        optional_params: &optional_params,
    })
    .expect("the fixture request builds");
    assert_eq!(body.input_type, Json::Text(String::from("chat")));
    assert_eq!(body.output_type, Json::Text(String::from("chat")));
    assert_eq!(body.session_id, None);
}

#[rstest]
#[case::input_type(r#"{"input_type":"text"}"#, "text", "chat")]
#[case::output_type(r#"{"output_type":"text"}"#, "chat", "text")]
#[case::empty_values(r#"{"input_type":"","output_type":""}"#, "", "")]
#[case::null_values(r#"{"input_type":null,"output_type":null}"#, "chat", "chat")]
fn the_body_keeps_the_type_parameters(
    #[case] options: &str,
    #[case] input_type: &str,
    #[case] output_type: &str,
) {
    let optional_params = object(options);
    let body = transform_request(&RequestInput {
        messages: &[],
        optional_params: &optional_params,
    })
    .expect("the fixture request builds");
    assert_eq!(body.input_type, Json::Text(String::from(input_type)));
    assert_eq!(body.output_type, Json::Text(String::from(output_type)));
}

#[rstest]
#[case::present(r#"{"session_id":"s1"}"#, Some("s1"))]
#[case::empty(r#"{"session_id":""}"#, None)]
#[case::zero(r#"{"session_id":0}"#, None)]
#[case::false_value(r#"{"session_id":false}"#, None)]
#[case::empty_list(r#"{"session_id":[]}"#, None)]
#[case::null(r#"{"session_id":null}"#, None)]
#[case::absent("{}", None)]
fn the_body_sends_only_a_truthy_session_id(#[case] options: &str, #[case] expected: Option<&str>) {
    let optional_params = object(options);
    let body = transform_request(&RequestInput {
        messages: &[],
        optional_params: &optional_params,
    })
    .expect("the fixture request builds");
    assert_eq!(
        body.session_id,
        expected.map(|value| Json::Text(String::from(value)))
    );
}

#[test]
fn tweaks_in_the_request_options_are_refused() {
    let error = transform_request(&RequestInput {
        messages: &[],
        optional_params: &object(r#"{"tweaks":{"Chat":"value"}}"#),
    })
    .expect_err("a request parameter cannot override the flow components");
    assert!(matches!(
        error,
        LangFlowError::Api { status_code: 400, message } if message == TWEAKS_MESSAGE
    ));
}

#[test]
fn a_null_tweaks_option_is_allowed() {
    let request = transform_request(&RequestInput {
        messages: &[],
        optional_params: &object(r#"{"tweaks":null}"#),
    })
    .expect("a null tweaks value is unset");
    assert_eq!(request.input_value, "");
}

#[test]
fn the_body_serializes_the_langflow_shape() {
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let optional_params = object(r#"{"input_type":"text","session_id":"s1"}"#);
    let request = transform_request(&RequestInput {
        messages: &messages,
        optional_params: &optional_params,
    })
    .expect("the fixture request builds");
    assert_eq!(
        serde_json::to_string(&request).expect("the body serializes"),
        r#"{"input_value":"hi","input_type":"text","output_type":"chat","session_id":"s1"}"#
    );
}
