#![cfg(test)]

use rstest::rstest;
use serde_json::{Map, Value, json};

use betterlite_models::full_model_metadata::parameters::SupportedParameterSource as _;

use super::deps::NoParameters;
use super::error::StructuredWriteBackError;
use super::{
    defer_tool_exchange_systems, fold_leading_systems, hoisted_system_message,
    system_row_to_anthropic, translate_to_openai, write_back_structured_messages,
};

const PROMPT: &str =
    r#"{"model": "gpt-4o", "system": "be nice", "messages": [{"role": "user", "content": "u"}]}"#;

fn data(raw: &str) -> Map<String, Value> {
    serde_json::from_str(raw).unwrap()
}

fn user_row(text: &str) -> Value {
    json!({"role": "user", "content": text})
}

fn system_row(text: &str) -> Value {
    json!({"role": "system", "content": text})
}

fn tool_call_row(id: &str) -> Value {
    json!({"role": "assistant", "content": null, "tool_calls": [
        {"id": id, "type": "function", "function": {"name": "f", "arguments": "{}"}}
    ]})
}

fn tool_result_row(id: &str, content: &str) -> Value {
    json!({"role": "tool", "tool_call_id": id, "content": content})
}

fn user_turn(text: &str) -> Value {
    json!({"role": "user", "content": [{"type": "text", "text": text}]})
}

fn anthropic_row(message: Value) -> Option<Value> {
    system_row_to_anthropic(&message).map(Value::Object)
}

#[tokio::test]
async fn hoisting_returns_the_first_message_for_a_string_system() {
    let request = data(r#"{"model": "gpt-4o", "system": "be nice"}"#);
    assert_eq!(
        hoisted_system_message(&request).await.unwrap(),
        Some(json!({"role": "system", "content": "be nice"}))
    );
}

#[tokio::test]
async fn hoisting_returns_the_system_turn_for_block_system() {
    let request = data(
        r#"{"model": "gpt-4o", "system": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}"#,
    );
    assert_eq!(
        hoisted_system_message(&request).await.unwrap(),
        Some(json!({"role": "system", "content": [
            {"type": "text", "text": "a"},
            {"type": "text", "text": "b"}
        ]}))
    );
}

#[rstest]
#[case::missing(r#"{"model": "gpt-4o"}"#)]
#[case::null(r#"{"model": "gpt-4o", "system": null}"#)]
#[case::empty_string(r#"{"model": "gpt-4o", "system": ""}"#)]
#[case::empty_list(r#"{"model": "gpt-4o", "system": []}"#)]
#[case::zero(r#"{"model": "gpt-4o", "system": 0}"#)]
#[case::false_value(r#"{"model": "gpt-4o", "system": false}"#)]
#[case::empty_object(r#"{"model": "gpt-4o", "system": {}}"#)]
#[tokio::test]
async fn hoisting_returns_none_for_a_missing_or_empty_system(#[case] raw: &str) {
    assert_eq!(hoisted_system_message(&data(raw)).await.unwrap(), None);
}

#[test]
fn system_conversion_keeps_a_non_empty_string() {
    assert_eq!(
        anthropic_row(json!({"role": "system", "content": "keep"})),
        Some(json!({"role": "system", "content": "keep"}))
    );
}

#[test]
fn system_conversion_returns_none_for_an_empty_string() {
    assert_eq!(
        anthropic_row(json!({"role": "system", "content": ""})),
        None
    );
}

#[rstest]
#[case::missing_key(json!({"role": "system"}))]
#[case::null(json!({"role": "system", "content": null}))]
#[case::empty_list(json!({"role": "system", "content": []}))]
#[case::object(json!({"role": "system", "content": {"text": "x"}}))]
#[test]
fn system_conversion_returns_none_for_content_of_another_type(#[case] message: Value) {
    assert_eq!(anthropic_row(message), None);
}

#[test]
fn system_conversion_keeps_only_text_blocks_and_copies_cache_control() {
    let message = json!({"role": "system", "content": [
        {"type": "text", "text": "keep", "cache_control": {"type": "ephemeral"}, "citations": []},
        {"type": "image", "source": {"type": "url", "url": "https://example.test/i.png"}},
        {"type": "text", "text": ""},
        {"type": "text", "text": null},
        {"type": "text"}
    ]});
    assert_eq!(
        anthropic_row(message),
        Some(json!({"role": "system", "content": [
            {"type": "text", "text": "keep", "cache_control": {"type": "ephemeral"}}
        ]}))
    );
}

#[test]
fn system_conversion_returns_none_when_no_block_survives() {
    let message = json!({"role": "system", "content": [
        {"type": "image", "source": {"type": "url", "url": "https://example.test/i.png"}},
        {"type": "text", "text": ""}
    ]});
    assert_eq!(anthropic_row(message), None);
}

#[test]
fn system_conversion_judges_cache_control_by_truthiness() {
    let message = json!({"role": "system", "content": [
        {"type": "text", "text": "null", "cache_control": null},
        {"type": "text", "text": "false", "cache_control": false},
        {"type": "text", "text": "empty-string", "cache_control": ""},
        {"type": "text", "text": "empty-list", "cache_control": []},
        {"type": "text", "text": "empty-object", "cache_control": {}},
        {"type": "text", "text": "zero", "cache_control": 0_i64},
        {"type": "text", "text": "one", "cache_control": 1_i64},
        {"type": "text", "text": "number", "cache_control": 2.5_f64},
        {"type": "text", "text": "text", "cache_control": "x"},
        {"type": "text", "text": "true", "cache_control": true}
    ]});
    assert_eq!(
        anthropic_row(message),
        Some(json!({"role": "system", "content": [
            {"type": "text", "text": "one", "cache_control": 1_i64},
            {"type": "text", "text": "number", "cache_control": 2.5_f64},
            {"type": "text", "text": "text", "cache_control": "x"},
            {"type": "text", "text": "true", "cache_control": true}
        ]}))
    );
}

#[test]
fn fold_puts_the_existing_system_first_only_when_asked() {
    let leading = vec![system_row("new")];
    let mut asked = data(r#"{"system": "old"}"#);
    fold_leading_systems(&mut asked, &leading, true);
    assert_eq!(
        asked.get("system"),
        Some(&json!([
            {"type": "text", "text": "old"},
            {"type": "text", "text": "new"}
        ]))
    );
    let mut not_asked = data(r#"{"system": "old"}"#);
    fold_leading_systems(&mut not_asked, &leading, false);
    assert_eq!(
        not_asked.get("system"),
        Some(&json!([{"type": "text", "text": "new"}]))
    );
}

#[test]
fn fold_keeps_an_existing_array_system_as_it_is() {
    let mut request = data(
        r#"{"system": [{"type": "text", "text": "old", "cache_control": {"type": "ephemeral"}}]}"#,
    );
    fold_leading_systems(&mut request, &[], true);
    assert_eq!(
        request.get("system"),
        Some(&json!([
            {"type": "text", "text": "old", "cache_control": {"type": "ephemeral"}}
        ]))
    );
}

#[test]
fn fold_appends_block_content_of_a_system_row() {
    let leading = vec![json!({"role": "system", "content": [
        {"type": "text", "text": "a"},
        {"type": "image", "source": {"type": "url", "url": "x"}},
        {"type": "text", "text": "b"}
    ]})];
    let mut request = data("{}");
    fold_leading_systems(&mut request, &leading, true);
    assert_eq!(
        request.get("system"),
        Some(&json!([
            {"type": "text", "text": "a"},
            {"type": "text", "text": "b"}
        ]))
    );
}

#[test]
fn fold_removes_the_system_key_when_nothing_is_left() {
    let mut request = data(r#"{"system": "", "messages": []}"#);
    fold_leading_systems(&mut request, &[system_row("")], true);
    assert_eq!(request.get("system"), None);
    assert_eq!(request.get("messages"), Some(&json!([])));
}

#[test]
fn fold_ignores_a_leading_row_that_does_not_convert() {
    let leading = vec![json!("plain string"), system_row("kept")];
    let mut request = data("{}");
    fold_leading_systems(&mut request, &leading, true);
    assert_eq!(
        request.get("system"),
        Some(&json!([{"type": "text", "text": "kept"}]))
    );
}

#[rstest]
#[case::null(json!(null))]
#[case::zero(json!(0_i64))]
#[case::false_value(json!(false))]
#[case::object(json!({"text": "x"}))]
#[test]
fn fold_ignores_an_existing_system_of_another_type(#[case] system: Value) {
    let mut request = Map::new();
    request.insert("system".to_owned(), system);
    fold_leading_systems(&mut request, &[system_row("new")], true);
    assert_eq!(
        request.get("system"),
        Some(&json!([{"type": "text", "text": "new"}]))
    );
}

#[test]
fn defer_holds_a_system_row_until_the_tool_result() {
    let messages = vec![
        tool_call_row("call_1"),
        system_row("note"),
        tool_result_row("call_1", "r"),
    ];
    assert_eq!(
        defer_tool_exchange_systems(&messages),
        vec![
            tool_call_row("call_1"),
            tool_result_row("call_1", "r"),
            system_row("note"),
        ]
    );
}

#[test]
fn defer_moves_the_system_row_after_two_results() {
    let messages = vec![
        tool_call_row("call_1"),
        system_row("note"),
        tool_result_row("call_1", "r1"),
        tool_result_row("call_1", "r2"),
    ];
    assert_eq!(
        defer_tool_exchange_systems(&messages),
        vec![
            tool_call_row("call_1"),
            tool_result_row("call_1", "r1"),
            tool_result_row("call_1", "r2"),
            system_row("note"),
        ]
    );
}

#[test]
fn defer_keeps_the_order_without_a_tool_exchange() {
    let messages = vec![user_row("u"), system_row("note"), user_row("u2")];
    assert_eq!(defer_tool_exchange_systems(&messages), messages);
}

#[test]
fn defer_flushes_at_the_end_of_the_exchange() {
    let messages = vec![
        tool_call_row("call_1"),
        system_row("note"),
        tool_result_row("call_1", "r"),
        user_row("after"),
    ];
    assert_eq!(
        defer_tool_exchange_systems(&messages),
        vec![
            tool_call_row("call_1"),
            tool_result_row("call_1", "r"),
            system_row("note"),
            user_row("after"),
        ]
    );
}

#[test]
fn defer_keeps_a_system_row_after_the_exchange_in_place() {
    let messages = vec![
        tool_call_row("call_1"),
        system_row("inside"),
        tool_result_row("call_1", "r"),
        system_row("outside"),
    ];
    assert_eq!(
        defer_tool_exchange_systems(&messages),
        vec![
            tool_call_row("call_1"),
            tool_result_row("call_1", "r"),
            system_row("inside"),
            system_row("outside"),
        ]
    );
}

#[test]
fn defer_flushes_each_exchange_before_the_next() {
    let messages = vec![
        tool_call_row("call_1"),
        system_row("inside-1"),
        tool_result_row("call_1", "r1"),
        tool_call_row("call_2"),
        system_row("inside-2"),
        tool_result_row("call_2", "r2"),
    ];
    assert_eq!(
        defer_tool_exchange_systems(&messages),
        vec![
            tool_call_row("call_1"),
            tool_result_row("call_1", "r1"),
            system_row("inside-1"),
            tool_call_row("call_2"),
            tool_result_row("call_2", "r2"),
            system_row("inside-2"),
        ]
    );
}

#[test]
fn defer_keeps_every_system_row_when_all_rows_are_system() {
    let messages = vec![system_row("one"), system_row("two")];
    assert_eq!(defer_tool_exchange_systems(&messages), messages);
}

#[tokio::test]
async fn structured_translation_preserves_a_midturn_system_row() {
    let request = data(
        r#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "u"}, {"role": "system", "content": "mid"}]}"#,
    );
    let translated = serde_json::to_value(translate_to_openai(&request).await.unwrap()).unwrap();
    assert_eq!(
        translated.get("messages"),
        Some(&json!([
            {"role": "user", "content": "u"},
            {"role": "system", "content": "mid"}
        ]))
    );
}

#[test]
fn structured_translation_declares_no_parameters() {
    assert_eq!(
        NoParameters.supported_openai_params("claude-sonnet-4-5", Some("anthropic")),
        Ok(None)
    );
}

#[tokio::test]
async fn structured_conversion_downloads_an_https_image_for_a_bedrock_model() {
    let returned = vec![json!({"role": "user", "content": [
        {"type": "image_url", "image_url": {"url": "https://example.test/a.png"}}
    ]})];

    let mut bedrock = data(r#"{"model": "invoke/anthropic.claude-v2", "messages": []}"#);
    let error = write_back_structured_messages(&mut bedrock, &returned, None, false)
        .await
        .unwrap_err();
    assert!(matches!(&error, StructuredWriteBackError::Conversion(_)));

    let mut plain = data(r#"{"model": "claude-sonnet-4-5", "messages": []}"#);
    write_back_structured_messages(&mut plain, &returned, None, false)
        .await
        .unwrap();
    assert_eq!(
        plain.get("messages"),
        Some(&json!([{"role": "user", "content": [
            {"type": "image", "source": {"type": "url", "url": "https://example.test/a.png"}}
        ]}]))
    );
}

#[tokio::test]
async fn structured_keeps_the_client_system_when_the_hoisted_prompt_is_unchanged() {
    let mut request = data(PROMPT);
    let hoisted = hoisted_system_message(&request).await.unwrap();
    let returned = vec![hoisted.clone().unwrap(), user_row("u")];
    write_back_structured_messages(&mut request, &returned, hoisted.as_ref(), false)
        .await
        .unwrap();
    assert_eq!(request.get("system"), Some(&json!("be nice")));
    assert_eq!(request.get("messages"), Some(&json!([user_turn("u")])));
}

#[tokio::test]
async fn structured_folds_a_rewritten_hoisted_prompt() {
    let mut request = data(PROMPT);
    let hoisted = hoisted_system_message(&request).await.unwrap();
    let returned = vec![system_row("be mean"), user_row("u")];
    write_back_structured_messages(&mut request, &returned, hoisted.as_ref(), false)
        .await
        .unwrap();
    assert_eq!(
        request.get("system"),
        Some(&json!([{"type": "text", "text": "be mean"}]))
    );
    assert_eq!(request.get("messages"), Some(&json!([user_turn("u")])));
}

#[tokio::test]
async fn structured_drops_the_hoisted_prompt_once() {
    let mut request = data(PROMPT);
    let hoisted = hoisted_system_message(&request).await.unwrap();
    let returned = vec![
        user_row("u"),
        system_row("be nice"),
        user_row("u2"),
        system_row("be nice"),
        user_row("u3"),
    ];
    write_back_structured_messages(&mut request, &returned, hoisted.as_ref(), true)
        .await
        .unwrap();
    assert_eq!(request.get("system"), Some(&json!("be nice")));
    assert_eq!(
        request.get("messages"),
        Some(&json!([
            user_turn("u"),
            user_turn("u2"),
            {"role": "system", "content": "be nice"},
            user_turn("u3")
        ]))
    );
}

#[tokio::test]
async fn structured_preserves_a_midturn_system_row() {
    let mut request =
        data(r#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "u"}]}"#);
    let returned = vec![
        user_row("u"),
        json!({"role": "assistant", "content": "a"}),
        system_row("reminder"),
        user_row("u2"),
    ];
    write_back_structured_messages(&mut request, &returned, None, true)
        .await
        .unwrap();
    assert_eq!(
        request.get("messages"),
        Some(&json!([
            user_turn("u"),
            {"role": "assistant", "content": [{"type": "text", "text": "a"}]},
            {"role": "system", "content": "reminder"},
            user_turn("u2")
        ]))
    );
    assert_eq!(request.get("system"), None);
}

#[tokio::test]
async fn structured_treats_a_mixed_case_system_role_as_system() {
    let mut request =
        data(r#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "u"}]}"#);
    let returned = vec![
        user_row("u"),
        json!({"role": "System", "content": "reminder"}),
        user_row("u2"),
    ];
    write_back_structured_messages(&mut request, &returned, None, true)
        .await
        .unwrap();
    assert_eq!(
        request.get("messages"),
        Some(&json!([
            user_turn("u"),
            {"role": "system", "content": "reminder"},
            user_turn("u2")
        ]))
    );
}

#[tokio::test]
async fn structured_drops_a_midturn_system_row_without_the_flag() {
    let mut request =
        data(r#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "u"}]}"#);
    let returned = vec![
        user_row("u"),
        json!({"role": "assistant", "content": "a"}),
        system_row("reminder"),
        user_row("u2"),
    ];
    write_back_structured_messages(&mut request, &returned, None, false)
        .await
        .unwrap();
    assert_eq!(
        request.get("messages"),
        Some(&json!([
            user_turn("u"),
            {"role": "assistant", "content": [{"type": "text", "text": "a"}]},
            user_turn("u2")
        ]))
    );
}

#[tokio::test]
async fn structured_adds_the_placeholder_when_only_system_rows_remain() {
    let mut request =
        data(r#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "u"}]}"#);
    let returned = vec![system_row("only")];
    write_back_structured_messages(&mut request, &returned, None, true)
        .await
        .unwrap();
    assert_eq!(
        request.get("system"),
        Some(&json!([{"type": "text", "text": "only"}]))
    );
    assert_eq!(
        request.get("messages"),
        Some(&json!([
            {"role": "user", "content": [{"type": "text", "text": "Please continue."}]}
        ]))
    );
}

#[tokio::test]
async fn structured_strips_cache_control_from_thinking_blocks() {
    let mut request = data(r#"{"model": "gpt-4o", "messages": []}"#);
    let returned = vec![
        user_row("u"),
        json!({"role": "assistant", "content": [
            {"type": "thinking", "thinking": "hmm", "signature": "sig", "cache_control": {"type": "ephemeral"}},
            {"type": "text", "text": "answer"}
        ]}),
    ];
    write_back_structured_messages(&mut request, &returned, None, false)
        .await
        .unwrap();
    assert_eq!(
        request.get("messages"),
        Some(&json!([
            user_turn("u"),
            {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "hmm", "signature": "sig"},
                {"type": "text", "text": "answer"}
            ]}
        ]))
    );
}

#[tokio::test]
async fn structured_converts_each_tool_exchange_group_on_its_own() {
    let mut request = data(r#"{"model": "gpt-4o", "messages": []}"#);
    let returned = vec![
        user_row("a"),
        tool_call_row("call_1"),
        tool_result_row("call_1", "r"),
        user_row("b"),
    ];
    write_back_structured_messages(&mut request, &returned, None, false)
        .await
        .unwrap();
    assert_eq!(
        request.get("messages"),
        Some(&json!([
            user_turn("a"),
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "call_1", "name": "f", "input": {}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "call_1", "content": "r"}
            ]},
            user_turn("b")
        ]))
    );
}

#[tokio::test]
async fn conversion_error_reports_a_non_object_row_and_keeps_the_messages() {
    let mut request =
        data(r#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "keep"}]}"#);
    let original = request.get("messages").cloned();
    let returned = vec![user_row("u"), json!("not a message")];
    let error = write_back_structured_messages(&mut request, &returned, None, false)
        .await
        .unwrap_err();
    assert!(matches!(&error, StructuredWriteBackError::Conversion(_)));
    assert_eq!(error.to_string(), "message at index 0 is not an object");
    assert_eq!(request.get("messages").cloned(), original);
}

#[tokio::test]
async fn conversion_error_reports_a_missing_role() {
    let mut request =
        data(r#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "keep"}]}"#);
    let original = request.get("messages").cloned();
    let returned = vec![user_row("u"), json!({"content": "x"})];
    let error = write_back_structured_messages(&mut request, &returned, None, false)
        .await
        .unwrap_err();
    assert!(matches!(&error, StructuredWriteBackError::Conversion(_)));
    assert_eq!(error.to_string(), "message at index 0 has no role");
    assert_eq!(request.get("messages").cloned(), original);
}
