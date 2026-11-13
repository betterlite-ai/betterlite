use crate::bedrock_processor::conversation::push_user_blocks;
use crate::bedrock_processor::error::ProcessorError;

use super::prelude::{Harness, RecordingCachePoints, Settings, capture, json, list};

const TOOL_MESSAGE: &str = r#"{"role":"tool","tool_call_id":"call_1","content":"x"}"#;

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_consecutive_user_turns_merge() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages =
        list(r#"[{"role":"user","content":"first"},{"role":"user","content":"second"}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(r#"[{"role":"user","content":[{"text":"first"},{"text":"second"}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_tool_turn_after_a_user_turn_merges() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(&format!(
        r#"[{{"role":"user","content":"hi"}},{TOOL_MESSAGE}]"#
    ));
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"text":"hi"},
                {"toolResult":{"content":[{"text":"x"}],"toolUseId":"call_1"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_tool_turn_splits_under_modify_params() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let settings = Settings::default().with_modify_params();
    let messages = list(&format!(
        r#"[{{"role":"user","content":"hi"}},{TOOL_MESSAGE}]"#
    ));
    let converted = harness.convert(&messages, &settings).await.unwrap();
    assert_eq!(
        converted,
        list(
            r#"[
                {"role":"user","content":[{"text":"hi"}]},
                {"role":"assistant","content":[{"text":"Please continue."}]},
                {"role":"user","content":[
                    {"toolResult":{"content":[{"text":"x"}],"toolUseId":"call_1"}}
                ]}
            ]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_assistant_continue_message_is_used_verbatim()
{
    let harness = Harness::new(RecordingCachePoints::accepting());
    let continue_message = json(r#"{"role":"assistant","content":"keep going"}"#);
    let settings = Settings::default().with_assistant_continue(&continue_message);
    let messages = list(&format!(
        r#"[{{"role":"user","content":"hi"}},{TOOL_MESSAGE}]"#
    ));
    let converted = harness.convert(&messages, &settings).await.unwrap();
    assert_eq!(
        converted,
        list(
            r#"[
                {"role":"user","content":[{"text":"hi"}]},
                {"role":"assistant","content":[{"text":"keep going"}]},
                {"role":"user","content":[
                    {"toolResult":{"content":[{"text":"x"}],"toolUseId":"call_1"}}
                ]}
            ]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_assistant_continue_string_is_used_verbatim() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let continue_message = json(r#""keep going""#);
    let settings = Settings::default().with_assistant_continue(&continue_message);
    let messages = list(&format!(
        r#"[{{"role":"user","content":"hi"}},{TOOL_MESSAGE}]"#
    ));
    let converted = harness.convert(&messages, &settings).await.unwrap();
    assert_eq!(
        converted,
        list(
            r#"[
                {"role":"user","content":[{"text":"hi"}]},
                {"role":"assistant","content":[{"text":"keep going"}]},
                {"role":"user","content":[
                    {"toolResult":{"content":[{"text":"x"}],"toolUseId":"call_1"}}
                ]}
            ]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_null_assistant_continue_message_still_merges()
{
    let harness = Harness::new(RecordingCachePoints::accepting());
    let continue_message = json("null");
    let settings = Settings::default().with_assistant_continue(&continue_message);
    let messages = list(&format!(
        r#"[{{"role":"user","content":"hi"}},{TOOL_MESSAGE}]"#
    ));
    let converted = harness.convert(&messages, &settings).await.unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"text":"hi"},
                {"toolResult":{"content":[{"text":"x"}],"toolUseId":"call_1"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_empty_assistant_turn_does_not_break_the_merge()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":"first"},
            {"role":"assistant","content":""},
            {"role":"user","content":"second"}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(r#"[{"role":"user","content":[{"text":"first"},{"text":"second"}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_system_turn_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages =
        list(r#"[{"role":"user","content":"hi"},{"role":"system","content":"be nice"}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::UnsupportedRole { message: 1, role } if role == "system"
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_function_turn_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"function","content":"hi"}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::UnsupportedRole { message: 0, role } if role == "function"
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_null_role_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":null,"content":"hi"}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::UnsupportedRole { message: 0, role } if role == "null"
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_message_without_a_role_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"content":"hi"}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::MessageWithoutRole { message: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_non_string_role_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":5,"content":"hi"}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(error, ProcessorError::RoleNotText { message: 0 }));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_repeating_a_conversion_keeps_the_input_and_result()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let settings = Settings::default().with_modify_params();
    let messages = list(r#"[{"role":"assistant","content":"draft"}]"#);
    let before = messages.clone();
    let first = harness.convert(&messages, &settings).await.unwrap();
    let second = harness.convert(&messages, &settings).await.unwrap();
    assert_eq!(
        first,
        list(
            r#"[
                {"role":"user","content":[{"text":"Please continue."}]},
                {"role":"assistant","content":[{"text":"draft"}]},
                {"role":"user","content":[{"text":"Please continue."}]}
            ]"#
        )
    );
    assert_eq!(second, first);
    assert_eq!(messages, before);
}

#[test]
fn bedrock_processor_behavior_tests_behavior_merging_rejects_content_that_is_not_a_list() {
    let mut contents = list(r#"[{"role":"user","content":"text"}]"#);
    let error = push_user_blocks(
        &mut contents,
        vec![json(r#"{"text":"later"}"#)],
        None,
        false,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::MergedContentNotList { message: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_merging_consecutive_blocks_is_logged() {
    let (captured, _guard) = capture();
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(&format!(
        r#"[{{"role":"user","content":"hi"}},{TOOL_MESSAGE}]"#
    ));
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"text":"hi"},
                {"toolResult":{"content":[{"text":"x"}],"toolUseId":"call_1"}}
            ]}]"#
        )
    );
    let logged = captured.text();
    assert!(logged.contains("consecutive user or tool blocks are merged"));
}
