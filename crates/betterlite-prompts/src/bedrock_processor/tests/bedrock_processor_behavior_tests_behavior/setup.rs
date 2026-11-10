use crate::bedrock_processor::error::ProcessorError;

use super::prelude::{Harness, RecordingCachePoints, Settings, json, list};

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_no_messages_under_modify_params_gets_the_default_user_turn()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let settings = Settings::default().with_modify_params();
    let converted = harness.convert(&[], &settings).await.unwrap();
    assert_eq!(
        converted,
        list(r#"[{"role":"user","content":[{"text":"Please continue."}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_no_messages_with_an_explicit_continue_message_uses_it()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let continue_message = json(r#"{"role":"user","content":"keep going"}"#);
    let settings = Settings::default().with_user_continue(&continue_message);
    let converted = harness.convert(&[], &settings).await.unwrap();
    assert_eq!(
        converted,
        list(r#"[{"role":"user","content":[{"text":"keep going"}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_no_messages_without_a_continue_message_is_rejected()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let error = harness
        .convert(&[], &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(error, ProcessorError::NoMessages));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_null_continue_message_counts_as_absent() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let continue_message = json("null");
    let settings = Settings::default().with_user_continue(&continue_message);
    let error = harness.convert(&[], &settings).await.unwrap_err();
    assert!(matches!(error, ProcessorError::NoMessages));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_leading_assistant_turn_gets_a_user_turn_before_it()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let settings = Settings::default().with_modify_params();
    let messages = list(r#"[{"role":"assistant","content":"draft"}]"#);
    let converted = harness.convert(&messages, &settings).await.unwrap();
    assert_eq!(
        converted,
        list(
            r#"[
                {"role":"user","content":[{"text":"Please continue."}]},
                {"role":"assistant","content":[{"text":"draft"}]},
                {"role":"user","content":[{"text":"Please continue."}]}
            ]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_leading_assistant_turn_with_prefix_keeps_its_place()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let settings = Settings::default().with_modify_params();
    let messages = list(r#"[{"role":"assistant","content":"draft","prefix":true}]"#);
    let converted = harness.convert(&messages, &settings).await.unwrap();
    assert_eq!(
        converted,
        list(r#"[{"role":"assistant","content":[{"text":"draft"}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_trailing_assistant_turn_gets_a_user_turn_after_it()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let settings = Settings::default().with_modify_params();
    let messages =
        list(r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"hello"}]"#);
    let converted = harness.convert(&messages, &settings).await.unwrap();
    assert_eq!(
        converted,
        list(
            r#"[
                {"role":"user","content":[{"text":"hi"}]},
                {"role":"assistant","content":[{"text":"hello"}]},
                {"role":"user","content":[{"text":"Please continue."}]}
            ]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_trailing_assistant_turn_with_prefix_is_left_alone()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let settings = Settings::default().with_modify_params();
    let messages = list(
        r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"hello","prefix":true}]"#,
    );
    let converted = harness.convert(&messages, &settings).await.unwrap();
    assert_eq!(
        converted,
        list(
            r#"[
                {"role":"user","content":[{"text":"hi"}]},
                {"role":"assistant","content":[{"text":"hello"}]}
            ]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_non_boolean_prefix_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"assistant","content":"draft","prefix":"yes"}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::PrefixNotBool { message: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_leading_assistant_gets_an_explicit_continue_user_turn()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let continue_message = json(r#"{"role":"user","content":"go on"}"#);
    let settings = Settings::default().with_user_continue(&continue_message);
    let messages = list(r#"[{"role":"assistant","content":"draft"}]"#);
    let converted = harness.convert(&messages, &settings).await.unwrap();
    assert_eq!(
        converted,
        list(
            r#"[
                {"role":"user","content":[{"text":"go on"}]},
                {"role":"assistant","content":[{"text":"draft"}]},
                {"role":"user","content":[{"text":"go on"}]}
            ]"#
        )
    );
}
