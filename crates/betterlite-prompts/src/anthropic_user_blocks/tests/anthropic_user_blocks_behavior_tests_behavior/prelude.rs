use std::iter::repeat_with;

use crate::anthropic_history::tools::MAX_HISTORY_MESSAGES;
use crate::anthropic_user_blocks::prelude::{PrepareError, prepare};
use crate::arguments::json::{Json, Object};
use crate::media::limits::{MAX_MESSAGE_PARTS, MAX_REMOTE_URLS};

use super::{messages, object};

const PLACEHOLDER: &str = "[System: Empty message content sanitised to satisfy protocol]";

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_an_empty_message_list_needs_modify_params() {
    let error = prepare(&[], "claude-sonnet-4-5", "anthropic", false).unwrap_err();
    assert!(matches!(error, PrepareError::NoUserMessages));
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_an_empty_message_list_with_modify_params_gets_the_dummy()
 {
    let prepared = prepare(&[], "claude-sonnet-4-5", "anthropic", true).unwrap();
    assert_eq!(
        prepared.messages,
        messages(r#"[{"role":"user","content":"Please continue."}]"#)
    );
    assert!(!prepared.force_base64);
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_modify_params_repairs_drop_an_orphan_tool_result()
{
    let input = messages(
        r#"[{"role":"tool","tool_call_id":"call","content":"hi"},{"role":"user","content":"go"}]"#,
    );
    let prepared = prepare(&input, "claude-sonnet-4-5", "anthropic", true).unwrap();
    assert_eq!(
        prepared.messages,
        messages(r#"[{"role":"user","content":"go"}]"#)
    );
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_modify_params_off_keeps_a_tool_result() {
    let input = messages(
        r#"[{"role":"tool","tool_call_id":"call","content":"hi"},{"role":"user","content":"go"}]"#,
    );
    let prepared = prepare(&input, "claude-sonnet-4-5", "anthropic", false).unwrap();
    assert_eq!(prepared.messages, input);
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_empty_text_is_rewritten_without_modify_params() {
    let prepared = prepare(
        &messages(r#"[{"role":"user","content":"  "}]"#),
        "claude-sonnet-4-5",
        "anthropic",
        false,
    )
    .unwrap();
    assert_eq!(
        prepared.messages,
        messages(&format!(r#"[{{"role":"user","content":"{PLACEHOLDER}"}}]"#))
    );
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_a_bedrock_invoke_model_forces_base64() {
    let prepared = prepare(&[], "invoke/anthropic.claude-v2", "bedrock", true).unwrap();
    assert!(prepared.force_base64);
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_a_vertex_provider_forces_base64() {
    let prepared = prepare(&[], "claude-sonnet-4-5", "vertex_ai-anthropic", true).unwrap();
    assert!(prepared.force_base64);
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_another_provider_keeps_image_urls() {
    let prepared = prepare(&[], "claude-sonnet-4-5", "anthropic", true).unwrap();
    assert!(!prepared.force_base64);
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_an_uppercase_invoke_model_forces_base64() {
    let prepared = prepare(&[], "INVOKE/anthropic.claude-v2", "bedrock", true).unwrap();
    assert!(prepared.force_base64);
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_a_remote_download_at_the_limit_is_allowed() {
    let input = vec![user_image_message(MAX_REMOTE_URLS)];
    let prepared = prepare(&input, "invoke/anthropic.claude-v2", "bedrock", false).unwrap();
    assert!(prepared.force_base64);
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_a_remote_download_past_the_limit_is_refused() {
    let input = vec![user_image_message(MAX_REMOTE_URLS), user_image_message(1)];
    let error = prepare(&input, "invoke/anthropic.claude-v2", "bedrock", false).unwrap_err();
    assert!(matches!(
        error,
        PrepareError::TooManyRemoteUrls {
            limit: MAX_REMOTE_URLS
        }
    ));
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_https_images_without_force_base64_are_not_downloads()
 {
    let input = vec![
        user_image_message(MAX_REMOTE_URLS),
        user_image_message(MAX_REMOTE_URLS),
    ];
    let prepared = prepare(&input, "claude-sonnet-4-5", "anthropic", false).unwrap();
    assert!(!prepared.force_base64);
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_too_many_messages_are_refused() {
    let input = repeat_with(|| user_message("go"))
        .take(MAX_HISTORY_MESSAGES + 1)
        .collect::<Vec<_>>();
    let error = prepare(&input, "claude-sonnet-4-5", "anthropic", false).unwrap_err();
    assert!(matches!(
        error,
        PrepareError::TooManyMessages {
            limit: MAX_HISTORY_MESSAGES
        }
    ));
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_too_many_parts_are_refused() {
    let parts = repeat_with(|| Json::Object(object(r#"{"type":"text","text":"a"}"#)))
        .take(MAX_MESSAGE_PARTS + 1)
        .collect::<Vec<_>>();
    let input = vec![Json::Object(Object::from_iter([
        ("role".to_owned(), Json::Text("user".to_owned())),
        ("content".to_owned(), Json::List(parts)),
    ]))];
    let error = prepare(&input, "claude-sonnet-4-5", "anthropic", false).unwrap_err();
    assert!(matches!(
        error,
        PrepareError::TooManyParts {
            limit: MAX_MESSAGE_PARTS
        }
    ));
}

#[test]
fn anthropic_user_blocks_behavior_tests_behavior_a_non_object_message_is_refused() {
    let error = prepare(
        &[Json::Text("not a message".to_owned())],
        "claude-sonnet-4-5",
        "anthropic",
        false,
    )
    .unwrap_err();
    assert!(matches!(error, PrepareError::MessageNotObject { index: 0 }));
}

fn user_message(text: &str) -> Json {
    Json::Object(object(&format!(r#"{{"role":"user","content":"{text}"}}"#)))
}

fn user_image_message(images: usize) -> Json {
    let parts = repeat_with(|| {
        Json::Object(object(
            r#"{"type":"image_url","image_url":"https://example.test/a.png"}"#,
        ))
    })
    .take(images)
    .collect::<Vec<_>>();
    Json::Object(Object::from_iter([
        ("role".to_owned(), Json::Text("user".to_owned())),
        ("content".to_owned(), Json::List(parts)),
    ]))
}
