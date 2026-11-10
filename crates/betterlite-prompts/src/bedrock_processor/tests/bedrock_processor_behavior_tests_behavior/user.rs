use crate::bedrock_content::error::ContentError;
use crate::bedrock_processor::error::ProcessorError;

use super::prelude::{DEFAULT_POINT, Harness, RecordingCachePoints, Settings, json, list};

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_user_text_turn_becomes_a_text_block() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":"hello"}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(r#"[{"role":"user","content":[{"text":"hello"}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_empty_user_text_turn_adds_no_block() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":""}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(converted, Vec::new());
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_whitespace_user_text_turn_is_kept() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":"   "}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(r#"[{"role":"user","content":[{"text":"   "}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_user_list_converts_every_supported_block() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[
            {"type":"text","text":"plain"},
            {"type":"guarded_text","text":"guarded"},
            {"type":"grounding_source","text":"source"},
            {"type":"query","text":"question"}
        ]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"text":"plain"},
                {"guardContent":{"text":{"text":"guarded"}}},
                {"text":"source"},
                {"text":"question"}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_unknown_user_block_adds_no_element() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages =
        list(r#"[{"role":"user","content":[{"type":"audio","audio":{"data":"QQ=="}}]}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(converted, Vec::new());
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_user_block_without_a_type_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"text":"plain"}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::BlockWithoutType { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_user_text_block_without_text_is_dropped() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"type":"text"}]}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(converted, Vec::new());
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_user_text_block_with_a_non_string_text_is_rejected()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"type":"text","text":5}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::Content(ContentError::TextPartNotText { index: 0 })
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_user_element_cache_control_adds_a_cache_point()
{
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"text","text":"x","cache_control":{"type":"ephemeral"}}]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"text":"x"},
                {"cachePoint":{"type":"default"}}
            ]}]"#
        )
    );
    assert_eq!(
        harness.cache.seen(),
        vec![(json(r#"{"type":"ephemeral"}"#), None)]
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_user_string_cache_control_adds_a_cache_point()
{
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":"x","cache_control":{"type":"ephemeral"}}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"text":"x"},
                {"cachePoint":{"type":"default"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_rejecting_cache_point_resolver_adds_no_block()
{
    let harness = Harness::new(RecordingCachePoints::rejecting());
    let messages = list(r#"[{"role":"user","content":"x","cache_control":{"type":"ephemeral"}}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(r#"[{"role":"user","content":[{"text":"x"}]}]"#)
    );
    assert_eq!(harness.cache.seen().len(), 1);
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_the_model_reaches_the_cache_point_resolver() {
    let harness = Harness::new(RecordingCachePoints::with_point(DEFAULT_POINT));
    let settings = Settings::default().with_model("anthropic.claude");
    let messages = list(r#"[{"role":"user","content":"x","cache_control":{"type":"ephemeral"}}]"#);
    let converted = harness.convert(&messages, &settings).await.unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"text":"x"},
                {"cachePoint":{"type":"default"}}
            ]}]"#
        )
    );
    assert_eq!(
        harness.cache.seen(),
        vec![(
            json(r#"{"type":"ephemeral"}"#),
            Some("anthropic.claude".to_owned())
        )]
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_user_content_of_another_type_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":5}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::ContentNotTextOrList { message: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_message_that_is_not_an_object_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list("[5]");
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::MessageNotObject { message: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_blank_user_text_with_modify_params_becomes_the_continue_text()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let settings = Settings::default().with_modify_params();
    let messages = list(r#"[{"role":"user","content":"  "}]"#);
    let converted = harness.convert(&messages, &settings).await.unwrap();
    assert_eq!(
        converted,
        list(r#"[{"role":"user","content":[{"text":"Please continue."}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_user_message_without_content_adds_no_block() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":null}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(converted, Vec::new());
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_unknown_user_block_still_takes_its_cache_point()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{
            "type":"audio","audio":{"data":"QQ=="},"cache_control":{"type":"ephemeral"}
        }]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"cachePoint":{"type":"default"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_user_image_without_an_image_url_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"type":"image_url"}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::ImageUrlMissing { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_grounding_source_without_text_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"type":"grounding_source"}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::TextWithoutText { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_grounding_source_with_a_non_string_text_is_rejected()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"type":"grounding_source","text":5}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(error, ProcessorError::TextNotText { block: 0 }));
}
