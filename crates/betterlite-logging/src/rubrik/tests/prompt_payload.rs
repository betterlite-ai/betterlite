use betterlite_logging::rubrik::prompt_payload::{
    PromptModerationInputs, PromptModerationRequest, build_prompt_moderation,
    flatten_moderation_messages,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
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
fn flatten_includes_tool_call_and_function_call_arguments_as_extra_segments() {
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
    ]));
    let flattened = flatten_moderation_messages(slice::from_ref(&message));
    assert_eq!(flattened[0].content, "thinking\n{\"city\":\"NY\"}");
}

#[test]
fn prompt_payload_includes_tools_only_when_present() {
    let inputs = PromptModerationInputs {
        model: Some("gpt-4"),
        structured_messages: &[],
        tools: None,
    };
    let request = PromptModerationRequest {
        model: None,
        user: None,
        correlation_key: None,
        litellm_call_id: None,
    };
    let payload = build_prompt_moderation(&inputs, &request);
    assert_eq!(payload.model, "gpt-4");
    assert_eq!(payload.tools, None);
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
fn prompt_payload_omits_correlation_key_when_both_sources_are_empty() {
    let inputs = PromptModerationInputs {
        model: Some("gpt-4"),
        structured_messages: &[],
        tools: None,
    };
    let request = PromptModerationRequest {
        model: None,
        user: None,
        correlation_key: Some(""),
        litellm_call_id: Some(""),
    };
    let payload = build_prompt_moderation(&inputs, &request);
    assert_eq!(payload.correlation_key, None);
}
