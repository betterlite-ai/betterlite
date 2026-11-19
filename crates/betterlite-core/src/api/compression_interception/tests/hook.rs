#![cfg(test)]

use betterlite_compression::compress::CompressionSkippedReason;
use betterlite_config::sections::compression_interception_state::Settings;
use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use super::support::{
    NOW, ScriptError, apply, disabled, fixed_clock, harness, index, list, number, object, request,
    request_with_tools, skipped_output, text,
};
use crate::api::compression_interception::hook::{CompressionHookError, HookResult};
use crate::routes::CallType;

#[tokio::test]
async fn a_disabled_hook_skips() {
    let harness = harness(disabled(), fixed_clock(NOW), Vec::new());
    let mut body = request();
    let result = apply(&harness, &mut body, Some(CallType::AnthropicMessages)).await;
    assert_eq!(result.expect("the hook succeeds"), HookResult::Skipped);
    assert!(harness.engine.calls().is_empty());
    assert_eq!(body, request());
}

#[rstest]
#[case::completion(CallType::Completion)]
#[case::embedding(CallType::Embedding)]
#[tokio::test]
async fn a_foreign_call_type_skips(#[case] call_type: CallType) {
    let harness = harness(Settings::default(), fixed_clock(NOW), Vec::new());
    let mut body = request();
    let result = apply(&harness, &mut body, Some(call_type)).await;
    assert_eq!(result.expect("the hook succeeds"), HookResult::Skipped);
    assert!(harness.engine.calls().is_empty());
}

#[tokio::test]
async fn an_absent_call_type_applies() {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        vec![Ok(skipped_output(
            CompressionSkippedReason::BelowTrigger,
            5,
        ))],
    );
    let mut body = request();
    let result = apply(&harness, &mut body, None).await;
    assert_eq!(result.expect("the hook succeeds"), HookResult::Applied);
    assert_eq!(harness.engine.calls().len(), 1);
}

#[rstest]
#[case::one(number(1))]
#[case::many(number(3))]
#[tokio::test]
async fn a_positive_agentic_depth_skips(#[case] depth: Json) {
    let harness = harness(Settings::default(), fixed_clock(NOW), Vec::new());
    let mut body = request();
    body.insert("_agentic_loop_depth".to_owned(), depth);
    let result = apply(&harness, &mut body, Some(CallType::AnthropicMessages)).await;
    assert_eq!(result.expect("the hook succeeds"), HookResult::Skipped);
    assert!(harness.engine.calls().is_empty());
}

#[rstest]
#[case::zero(number(0))]
#[case::negative(number(-2))]
#[case::null(Json::Null)]
#[case::not_a_number(text("two"))]
#[case::numeric_text(text("2"))]
#[case::not_whole(Json::Number(Number::from_f64(1.5).expect("finite")))]
#[case::boolean(Json::Bool(true))]
#[tokio::test]
async fn a_depth_that_is_not_a_positive_whole_number_applies(#[case] depth: Json) {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        vec![Ok(skipped_output(
            CompressionSkippedReason::BelowTrigger,
            5,
        ))],
    );
    let mut body = request();
    body.insert("_agentic_loop_depth".to_owned(), depth);
    let result = apply(&harness, &mut body, Some(CallType::AnthropicMessages)).await;
    assert_eq!(result.expect("the hook succeeds"), HookResult::Applied);
    assert_eq!(harness.engine.calls().len(), 1);
}

#[rstest]
#[case::object(object(vec![("role", text("user"))]))]
#[case::text(text("messages"))]
#[tokio::test]
async fn a_non_list_messages_field_skips(#[case] messages: Json) {
    let harness = harness(Settings::default(), fixed_clock(NOW), Vec::new());
    let mut body = request();
    body.insert("messages".to_owned(), messages);
    let result = apply(&harness, &mut body, Some(CallType::AnthropicMessages)).await;
    assert_eq!(result.expect("the hook succeeds"), HookResult::Skipped);
    assert!(harness.engine.calls().is_empty());
}

#[tokio::test]
async fn an_absent_messages_field_skips() {
    let harness = harness(Settings::default(), fixed_clock(NOW), Vec::new());
    let mut body = request();
    body.shift_remove("messages");
    let result = apply(&harness, &mut body, Some(CallType::AnthropicMessages)).await;
    assert_eq!(result.expect("the hook succeeds"), HookResult::Skipped);
    assert!(harness.engine.calls().is_empty());
}

#[rstest]
#[case::number(number(7))]
#[case::object(object(vec![("id", text("claude"))]))]
#[tokio::test]
async fn a_non_text_model_field_skips(#[case] model: Json) {
    let harness = harness(Settings::default(), fixed_clock(NOW), Vec::new());
    let mut body = request();
    body.insert("model".to_owned(), model);
    let result = apply(&harness, &mut body, Some(CallType::AnthropicMessages)).await;
    assert_eq!(result.expect("the hook succeeds"), HookResult::Skipped);
    assert!(harness.engine.calls().is_empty());
}

#[tokio::test]
async fn an_absent_model_field_skips() {
    let harness = harness(Settings::default(), fixed_clock(NOW), Vec::new());
    let mut body = request();
    body.shift_remove("model");
    let result = apply(&harness, &mut body, Some(CallType::AnthropicMessages)).await;
    assert_eq!(result.expect("the hook succeeds"), HookResult::Skipped);
    assert!(harness.engine.calls().is_empty());
}

#[tokio::test]
async fn an_empty_model_reaches_the_engine() {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        vec![Ok(skipped_output(
            CompressionSkippedReason::BelowTrigger,
            5,
        ))],
    );
    let mut body = request();
    body.insert("model".to_owned(), text(""));
    let result = apply(&harness, &mut body, Some(CallType::AnthropicMessages)).await;
    assert_eq!(result.expect("the hook succeeds"), HookResult::Applied);
    assert_eq!(harness.engine.calls()[0].model, "");
}

#[tokio::test]
async fn an_empty_messages_list_reaches_the_engine() {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        vec![Ok(skipped_output(
            CompressionSkippedReason::BelowTrigger,
            0,
        ))],
    );
    let mut body = request();
    body.insert("messages".to_owned(), list(Vec::new()));
    let result = apply(&harness, &mut body, Some(CallType::AnthropicMessages)).await;
    assert_eq!(result.expect("the hook succeeds"), HookResult::Applied);
    assert!(harness.engine.calls()[0].messages.is_empty());
}

#[tokio::test]
async fn an_existing_retrieval_tool_skips() {
    let harness = harness(Settings::default(), fixed_clock(NOW), Vec::new());
    let mut body = request_with_tools(list(vec![object(vec![
        ("type", text("custom")),
        ("name", text("litellm_content_retrieve")),
    ])]));
    let result = apply(&harness, &mut body, Some(CallType::AnthropicMessages)).await;
    assert_eq!(result.expect("the hook succeeds"), HookResult::Skipped);
    assert!(harness.engine.calls().is_empty());
}

#[tokio::test]
async fn the_engine_receives_the_request_and_the_configured_parameters() {
    let settings = Settings {
        enabled: true,
        compression_trigger: 12,
        compression_target: Some(7),
        embedding_model: Some("bedrock-titan".to_owned()),
        embedding_model_params: Some(index(vec![("dimensions", number(256))])),
    };
    let harness = harness(
        settings,
        fixed_clock(NOW),
        vec![Ok(skipped_output(
            CompressionSkippedReason::BelowTrigger,
            5,
        ))],
    );
    let mut body = request();
    let expected_messages = match body.get("messages") {
        Some(Json::List(messages)) => messages.clone(),
        _ => panic!("the fixture has messages"),
    };
    let result = apply(&harness, &mut body, Some(CallType::AnthropicMessages)).await;
    assert_eq!(result.expect("the hook succeeds"), HookResult::Applied);
    let calls = harness.engine.calls();
    let call = calls.first().expect("the engine ran once");
    assert_eq!(call.model, "claude-sonnet-4");
    assert_eq!(call.call_type, CallType::AnthropicMessages);
    assert_eq!(call.messages, expected_messages);
    assert_eq!(call.compression_trigger, 12);
    assert_eq!(call.compression_target, Some(7));
    assert_eq!(call.embedding_model.as_deref(), Some("bedrock-titan"));
    assert_eq!(
        call.embedding_model_params,
        Some(index(vec![("dimensions", number(256))]))
    );
}

#[tokio::test]
async fn an_engine_failure_is_returned() {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        vec![Err(ScriptError)],
    );
    let mut body = request();
    let result = apply(&harness, &mut body, Some(CallType::AnthropicMessages)).await;
    assert!(matches!(result, Err(CompressionHookError::Engine(_))));
    assert_eq!(body, request());
}
