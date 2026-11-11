use crate::bedrock_content::error::ContentError;
use crate::bedrock_processor::error::ProcessorError;

use super::prelude::{Harness, RecordingCachePoints, Settings, json, list};

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_assistant_text_turn_becomes_a_text_block() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages =
        list(r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"hello"}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
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
async fn bedrock_processor_behavior_tests_behavior_an_empty_assistant_text_turn_adds_no_block() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":"hi"},{"role":"assistant","content":""}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(r#"[{"role":"user","content":[{"text":"hi"}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_blank_assistant_text_blocks_are_dropped() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":"hi"},{"role":"assistant","content":[
            {"type":"text","text":"   "},
            {"type":"text","text":" kept "}
        ]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[
                {"role":"user","content":[{"text":"hi"}]},
                {"role":"assistant","content":[{"text":" kept "}]}
            ]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_assistant_image_ignores_the_object_format() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":"hi"},{"role":"assistant","content":[
            {"type":"image_url","image_url":{
                "url":"data:image/png;base64,AAAA",
                "format":"image/jpeg"
            }}
        ]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[
                {"role":"user","content":[{"text":"hi"}]},
                {"role":"assistant","content":[
                    {"image":{"format":"png","source":{"bytes":"AAAA"}}}
                ]}
            ]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_signed_thinking_element_becomes_reasoning_content()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"assistant","content":[
            {"type":"thinking","thinking":"why","signature":"sig"}
        ]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"assistant","content":[
                {"reasoningContent":{"reasoningText":{"text":"why","signature":"sig"}}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_unsigned_thinking_element_becomes_text() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages =
        list(r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"why"}]}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(r#"[{"role":"assistant","content":[{"text":"why"}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_blank_unsigned_thinking_element_is_dropped() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages =
        list(r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"  "}]}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(converted, Vec::new());
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_message_thinking_blocks_come_before_the_content()
{
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"assistant","content":"answer","thinking_blocks":[
            {"thinking":"earlier","signature":"sig"}
        ]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"assistant","content":[
                {"reasoningContent":{"reasoningText":{"text":"earlier","signature":"sig"}}},
                {"text":"answer"}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_message_thinking_blocks_that_are_not_a_list_are_rejected()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"assistant","content":"answer","thinking_blocks":{}}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::ThinkingBlocksNotList { message: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_assistant_tool_calls_become_tool_use_blocks() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":"weather?"},{"role":"assistant","content":null,"tool_calls":[
            {"id":"call_1","type":"function","function":{
                "name":"get_weather","arguments":"{\"city\":\"Paris\"}"
            }}
        ]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[
                {"role":"user","content":[{"text":"weather?"}]},
                {"role":"assistant","content":[
                    {"toolUse":{"input":{"city":"Paris"},"name":"get_weather","toolUseId":"call_1"}}
                ]}
            ]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_empty_tool_call_list_adds_no_block() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"assistant","content":"answer","tool_calls":[]}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(r#"[{"role":"assistant","content":[{"text":"answer"}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_tool_call_cache_control_adds_a_cache_point() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"assistant","content":null,"tool_calls":[
            {"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"},
             "cache_control":{"type":"ephemeral"}}
        ]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"assistant","content":[
                {"toolUse":{"input":{},"name":"f","toolUseId":"call_1"}},
                {"cachePoint":{"type":"default"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_tool_calls_that_are_not_a_list_are_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"assistant","content":"answer","tool_calls":{}}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::ToolCallsNotList { message: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_consecutive_assistant_turns_merge_and_sort() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":"go"},
            {"role":"assistant","content":null,"tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}
            ]},
            {"role":"assistant","content":"later"}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[
                {"role":"user","content":[{"text":"go"}]},
                {"role":"assistant","content":[
                    {"text":"later"},
                    {"toolUse":{"input":{},"name":"f","toolUseId":"call_1"}}
                ]}
            ]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_duplicate_tool_use_ids_keep_the_first_block() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"assistant","content":null,"tool_calls":[
                {"id":"dup","type":"function","function":{"name":"first","arguments":"{}"}}
            ]},
            {"role":"assistant","content":null,"tool_calls":[
                {"id":"dup","type":"function","function":{"name":"second","arguments":"{}"}}
            ]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"assistant","content":[
                {"toolUse":{"input":{},"name":"first","toolUseId":"dup"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_assistant_content_of_another_type_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"assistant","content":5}]"#);
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
async fn bedrock_processor_behavior_tests_behavior_an_assistant_element_that_is_not_an_object_is_rejected()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"assistant","content":[5]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::Content(ContentError::PartNotObject { index: 0 })
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_failed_tool_call_conversion_is_reported() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"assistant","content":null,"tool_calls":[
            {"type":"function","function":{"name":"f","arguments":"{}"}}
        ]}]"#,
    );
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(error, ProcessorError::ToolInvoke { .. }));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_assistant_string_cache_control_adds_a_cache_point()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages =
        list(r#"[{"role":"assistant","content":"answer","cache_control":{"type":"ephemeral"}}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"assistant","content":[
                {"text":"answer"},
                {"cachePoint":{"type":"default"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_blank_assistant_string_adds_no_cache_point() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages =
        list(r#"[{"role":"assistant","content":" ","cache_control":{"type":"ephemeral"}}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(converted, Vec::new());
    assert_eq!(harness.cache.seen().len(), 0);
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_assistant_element_cache_control_adds_a_cache_point()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"assistant","content":[
            {"type":"text","text":"answer","cache_control":{"type":"ephemeral"}}
        ]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"assistant","content":[
                {"text":"answer"},
                {"cachePoint":{"type":"default"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_assistant_message_without_content_adds_no_block()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"assistant","content":null}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(converted, Vec::new());
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_control_only_text_block_is_blank() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"assistant","content":[{"type":"text","text":"\u001c"}]}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(converted, Vec::new());
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_unknown_assistant_block_still_takes_its_cache_point()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"assistant","content":[{
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
            r#"[{"role":"assistant","content":[
                {"cachePoint":{"type":"default"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_blank_assistant_continue_text_is_dropped() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let continue_message = json(r#"{"role":"assistant","content":[{"type":"text","text":""}]}"#);
    let settings = Settings::default().with_assistant_continue(&continue_message);
    let messages = list(r#"[{"role":"assistant","content":[]}]"#);
    let converted = harness.convert(&messages, &settings).await.unwrap();
    assert_eq!(converted, Vec::new());
}
