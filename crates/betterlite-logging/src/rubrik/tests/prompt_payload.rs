use betterlite_logging::rubrik::prompt_payload::{
    PromptModerationInputs, PromptModerationPayload, PromptModerationRequest,
    build_prompt_moderation, flatten_moderation_messages,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use std::slice;

fn text_message(role: &str, content: &str) -> Json {
    Json::Object(IndexMap::from([
        ("role".to_owned(), Json::Text(role.to_owned())),
        ("content".to_owned(), Json::Text(content.to_owned())),
    ]))
}

fn content_part_message(role: &str, text: &str) -> Json {
    Json::Object(IndexMap::from([
        ("role".to_owned(), Json::Text(role.to_owned())),
        (
            "content".to_owned(),
            Json::List(vec![Json::Object(IndexMap::from([
                ("type".to_owned(), Json::Text("text".to_owned())),
                ("text".to_owned(), Json::Text(text.to_owned())),
            ]))]),
        ),
    ]))
}

#[test]
fn flatten_skips_non_dict_messages_and_keeps_role_and_text_content() {
    let messages = [
        text_message("user", "hello"),
        Json::Text("not a message".to_owned()),
        Json::Null,
    ];
    let flattened = flatten_moderation_messages(&messages);
    assert_eq!(flattened.len(), 1);
    assert_eq!(flattened[0].role, Some("user".to_owned()));
    assert_eq!(flattened[0].content, "hello");
}

#[test]
fn flatten_joins_anthropic_content_part_arrays_into_plain_text() {
    let messages = [content_part_message("user", "from a content part")];
    let flattened = flatten_moderation_messages(&messages);
    assert_eq!(flattened[0].content, "from a content part");
}

#[test]
fn flatten_joins_content_tool_call_and_function_call_arguments() {
    let message = Json::Object(IndexMap::from([
        ("role".to_owned(), Json::Text("assistant".to_owned())),
        ("content".to_owned(), Json::Text("thinking".to_owned())),
        (
            "tool_calls".to_owned(),
            Json::List(vec![Json::Object(IndexMap::from([(
                "function".to_owned(),
                Json::Object(IndexMap::from([(
                    "arguments".to_owned(),
                    Json::Text("{\"city\":\"NY\"}".to_owned()),
                )])),
            )]))]),
        ),
        (
            "function_call".to_owned(),
            Json::Object(IndexMap::from([(
                "arguments".to_owned(),
                Json::Text("{\"q\":\"x\"}".to_owned()),
            )])),
        ),
    ]));
    let flattened = flatten_moderation_messages(slice::from_ref(&message));
    assert_eq!(
        flattened[0].content,
        "thinking\n{\"city\":\"NY\"}\n{\"q\":\"x\"}"
    );
}

#[test]
fn flatten_appends_search_results_text_to_the_message_content() {
    let message = Json::Object(IndexMap::from([
        ("role".to_owned(), Json::Text("tool".to_owned())),
        ("content".to_owned(), Json::Text("result".to_owned())),
        (
            "search_results".to_owned(),
            Json::List(vec![Json::Object(IndexMap::from([
                ("source".to_owned(), Json::Text("src".to_owned())),
                ("title".to_owned(), Json::Text("Doc".to_owned())),
                (
                    "content".to_owned(),
                    Json::List(vec![Json::Object(IndexMap::from([
                        ("type".to_owned(), Json::Text("text".to_owned())),
                        ("text".to_owned(), Json::Text("body".to_owned())),
                    ]))]),
                ),
                (
                    "citations".to_owned(),
                    Json::Object(IndexMap::from([("enabled".to_owned(), Json::Bool(true))])),
                ),
            ]))]),
        ),
    ]));
    let flattened = flatten_moderation_messages(slice::from_ref(&message));
    assert_eq!(flattened[0].content, "resultsrcDocbody{\"enabled\":true}");
}

#[test]
fn prompt_payload_forwards_an_explicit_empty_tools_list() {
    let inputs = PromptModerationInputs {
        model: Some("gpt-4"),
        structured_messages: &[],
        tools: Some(Json::List(vec![])),
    };
    let request = PromptModerationRequest {
        model: None,
        user: None,
        correlation_key: None,
        litellm_call_id: None,
    };
    let payload = build_prompt_moderation(&inputs, &request);
    assert_eq!(payload.tools, Some(Json::List(vec![])));
}

fn assert_prompt_payload_json(payload: &PromptModerationPayload, expected: &str) {
    let serialized = serde_json::to_string(payload).expect("payload serializes");
    let actual_json = serde_json::from_str::<Json>(&serialized).expect("payload is JSON");
    let expected_json = serde_json::from_str::<Json>(expected).expect("expected value is JSON");
    assert_eq!(actual_json, expected_json);
}

#[rstest]
#[case::all_unset(None, None, None, None, r#"{"model":"gpt-4","messages":[]}"#)]
#[case::empty_strings(
    None,
    Some(""),
    Some(""),
    Some(""),
    r#"{"model":"gpt-4","messages":[]}"#
)]
#[case::all_set(Some(Json::List(vec![])), Some("user-1"), Some("ck"), None, r#"{"model":"gpt-4","messages":[],"tools":[],"user":"user-1","correlation_key":"ck"}"#)]
fn prompt_payload_serializes_only_the_optional_keys_it_has(
    #[case] tools: Option<Json>,
    #[case] user: Option<&str>,
    #[case] correlation_key: Option<&str>,
    #[case] litellm_call_id: Option<&str>,
    #[case] expected: &str,
) {
    let inputs = PromptModerationInputs {
        model: Some("gpt-4"),
        structured_messages: &[],
        tools,
    };
    let request = PromptModerationRequest {
        model: None,
        user,
        correlation_key,
        litellm_call_id,
    };
    let payload = build_prompt_moderation(&inputs, &request);
    assert_prompt_payload_json(&payload, expected);
}

#[test]
fn prompt_payload_falls_back_to_litellm_call_id_for_correlation_key() {
    let inputs = PromptModerationInputs {
        model: None,
        structured_messages: &[],
        tools: None,
    };
    let request = PromptModerationRequest {
        model: Some("gpt-4"),
        user: Some("user-1"),
        correlation_key: None,
        litellm_call_id: Some("call-123"),
    };
    let payload = build_prompt_moderation(&inputs, &request);
    assert_eq!(payload.correlation_key, Some("call-123".to_owned()));
    assert_eq!(payload.user, Some("user-1".to_owned()));
    assert_eq!(payload.model, "gpt-4");
}

#[test]
fn prompt_payload_prefers_the_inputs_model_and_an_explicit_correlation_key() {
    let inputs = PromptModerationInputs {
        model: Some("a"),
        structured_messages: &[],
        tools: None,
    };
    let request = PromptModerationRequest {
        model: Some("b"),
        user: None,
        correlation_key: Some("ck"),
        litellm_call_id: Some("call-1"),
    };
    let payload = build_prompt_moderation(&inputs, &request);
    assert_eq!(payload.model, "a");
    assert_eq!(payload.correlation_key, Some("ck".to_owned()));
}
