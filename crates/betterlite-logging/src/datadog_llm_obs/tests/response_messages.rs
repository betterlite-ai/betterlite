use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{json, serialized};
use crate::datadog_llm_obs::literal::literal_json;
use crate::datadog_llm_obs::response_messages::response_messages;

fn projected(response: &Json, call_type: Option<&str>) -> String {
    serialized(&response_messages(Some(response), call_type))
}

#[test]
fn reads_a_pretty_printed_python_literal_response_string() {
    let response = Json::Text(
        "{'choices': [{'message': {'content': 'word word word word word word word '\n                                    'word word word word',\n                          'role': 'assistant',\n                          'tool_calls': [{'function': {'arguments': '{\"a\": 1}',\n                                                       'name': 'f'},\n                                          'id': 'call_9',\n                                          'type': 'function'}]}}]}\n"
            .to_owned(),
    );
    assert_eq!(
        projected(&response, Some("acompletion")),
        r#"[{"role":"assistant","content":"word word word word word word word word word word word","tool_calls":[{"name":"f","arguments":{"a":1},"tool_id":"call_9","type":"function"}]}]"#
    );
}

#[rstest]
#[case::non_string_key("{1: 2}")]
#[case::bytes("b'a'")]
#[case::wrapped_bytes("b'abcd '\n      b'abcd'")]
#[case::complex("1j")]
#[case::empty_set("set()")]
#[case::surrogate(r"'\ud83d'")]
#[case::unicode_name(r"'\N{SNOWMAN}'")]
#[case::infinite_float("1e400")]
#[case::integer_past_u128("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF")]
fn a_sibling_value_json_cannot_hold_still_projects_the_message(#[case] sibling: &str) {
    let literal = format!(
        "{{'choices': [{{'message': {{'role': 'assistant', 'content': 'x'}}}}], 'z': {sibling}}}"
    );
    assert_eq!(
        projected(&Json::Text(literal), Some("completion")),
        r#"[{"role":"assistant","content":"x"}]"#
    );
}

#[test]
fn falls_back_to_json_when_the_literal_fails() {
    let response = Json::Text(
        r#"{"choices": [{"message": {"role": "assistant", "content": [{"type": "image_url", "image_url": {"url": "https://example.com/c.png", "detail": null}}]}}]}"#
            .to_owned(),
    );
    assert_eq!(
        projected(&response, Some("completion")),
        r#"[{"role":"assistant","content":[{"type":"image_url","image_url":{"url":"https://example.com/c.png","detail":null}}]}]"#
    );
}

#[test]
fn projects_a_response_that_is_already_an_object() {
    assert_eq!(
        projected(
            &json(r#"{"choices": [{"message": {"role": "assistant", "content": "hi"}}]}"#),
            Some("completion")
        ),
        r#"[{"role":"assistant","content":"hi"}]"#
    );
}

#[rstest]
#[case::completion("completion")]
#[case::acompletion("acompletion")]
#[case::text_completion("text_completion")]
#[case::atext_completion("atext_completion")]
#[case::generate_content("generate_content")]
#[case::agenerate_content("agenerate_content")]
#[case::generate_content_stream("generate_content_stream")]
#[case::agenerate_content_stream("agenerate_content_stream")]
#[case::anthropic_messages("anthropic_messages")]
fn every_llm_call_type_projects_its_response_message(#[case] call_type: &str) {
    assert_eq!(
        projected(
            &json(r#"{"choices": [{"message": {"role": "assistant", "content": "hi"}}]}"#),
            Some(call_type)
        ),
        r#"[{"role":"assistant","content":"hi"}]"#
    );
}

#[rstest]
#[case::embedding(Some("embedding"))]
#[case::responses(Some("responses"))]
#[case::unknown(Some("made_up_call"))]
#[case::empty(Some(""))]
#[case::absent(None)]
fn other_call_types_project_nothing(#[case] call_type: Option<&str>) {
    assert_eq!(
        projected(
            &json(r#"{"choices": [{"message": {"role": "assistant", "content": "hi"}}]}"#),
            call_type
        ),
        "[]"
    );
}

#[rstest]
#[case::no_choices("{}")]
#[case::empty_choices(r#"{"choices": []}"#)]
#[case::choices_not_a_list(r#"{"choices": {"message": {}}}"#)]
#[case::choice_not_an_object(r#"{"choices": ["message"]}"#)]
#[case::first_choice_without_a_message(r#"{"choices": [{"finish_reason": "stop"}]}"#)]
fn a_response_without_a_first_choice_message_projects_nothing(#[case] response: &str) {
    assert_eq!(projected(&json(response), Some("completion")), "[]");
}

#[test]
fn an_absent_or_null_response_projects_nothing() {
    assert_eq!(
        serialized(&response_messages(None, Some("completion"))),
        "[]"
    );
    assert_eq!(projected(&json("null"), Some("completion")), "[]");
}

#[rstest]
#[case::unparsed(r#"{"choices": [}"#)]
#[case::non_object_literal("'just text'")]
#[case::non_object_json("[1, 2]")]
fn a_response_string_that_is_not_an_object_projects_nothing(#[case] text: &str) {
    assert_eq!(
        projected(&Json::Text(text.to_owned()), Some("completion")),
        "[]"
    );
}

#[test]
fn a_deeply_nested_literal_is_refused_by_the_decoder() {
    let nested = format!("{}1{}", "[".repeat(200), "]".repeat(200));
    assert_eq!(literal_json(&nested), None);
    assert_eq!(projected(&Json::Text(nested), Some("completion")), "[]");
}

#[test]
fn a_literal_nested_under_the_cap_still_projects() {
    let content = (0_i32..58_i32).fold(Json::Text("deep".to_owned()), |inner, _| {
        Json::List(vec![inner])
    });
    let literal = format!(
        "{{'choices': [{{'message': {{'role': 'assistant', 'content': {}}}}}]}}",
        nested_python_literal(&content)
    );
    assert!(projected(&Json::Text(literal), Some("completion")).contains("deep"));
}

fn nested_python_literal(value: &Json) -> String {
    match value {
        Json::List(items) => format!(
            "[{}]",
            items
                .iter()
                .map(nested_python_literal)
                .collect::<Vec<String>>()
                .join(", ")
        ),
        Json::Text(text) => format!("'{text}'"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            panic!("the fixture holds only lists and text")
        }
    }
}
