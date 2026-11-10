use crate::bedrock_processor::error::ProcessorError;

use super::prelude::{Harness, RecordingCachePoints, Settings, json, list};

const TOOL_CALL: &str = r#"{"role":"assistant","content":null,"tool_calls":[
    {"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}
]}"#;

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_tool_text_message_becomes_a_tool_result() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(&format!(
        r#"[{{"role":"user","content":"hi"}},{TOOL_CALL},
            {{"role":"tool","tool_call_id":"call_1","content":"sunny"}}]"#
    ));
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
                    {"toolUse":{"input":{},"name":"f","toolUseId":"call_1"}}
                ]},
                {"role":"user","content":[
                    {"toolResult":{"content":[{"text":"sunny"}],"toolUseId":"call_1"}}
                ]}
            ]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_tool_result_media_blocks_are_converted() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"tool","tool_call_id":"call_1","content":[
            {"type":"text","text":"ok"},
            {"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}
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
                {"toolResult":{"content":[
                    {"text":"ok"},
                    {"image":{"format":"png","source":{"bytes":"AAAA"}}}
                ],"toolUseId":"call_1"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_tool_search_results_are_marked_successful() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"tool","tool_call_id":"call_1","content":"ignored","search_results":[
            {"source":"docs","title":"Title","content":[{"text":"body"}]}
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
                {"toolResult":{"content":[
                    {"searchResult":{
                        "source":"docs","title":"Title","content":[{"text":"body"}]
                    }}
                ],"toolUseId":"call_1","status":"success"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_tool_result_without_content_is_empty() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"tool","tool_call_id":"call_1"}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"toolResult":{"content":[],"toolUseId":"call_1"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_tool_message_cache_control_adds_a_cache_point()
{
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"tool","tool_call_id":"call_1","content":"x",
            "cache_control":{"type":"ephemeral"}}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"toolResult":{"content":[{"text":"x"}],"toolUseId":"call_1"}},
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
async fn bedrock_processor_behavior_tests_behavior_a_tool_content_cache_control_is_used() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"tool","tool_call_id":"call_1","content":[
            {"type":"text","text":"x","cache_control":{"type":"ephemeral"}}
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
                {"toolResult":{"content":[{"text":"x"}],"toolUseId":"call_1"}},
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
async fn bedrock_processor_behavior_tests_behavior_a_message_level_tool_cache_control_wins() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"tool","tool_call_id":"call_1","content":[
            {"type":"text","text":"x","cache_control":{"type":"ephemeral","ttl":"5m"}}
        ],"cache_control":{"type":"ephemeral"}}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"toolResult":{"content":[{"text":"x"}],"toolUseId":"call_1"}},
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
async fn bedrock_processor_behavior_tests_behavior_duplicate_tool_results_keep_the_first() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"tool","tool_call_id":"call_1","content":"first"},
            {"role":"tool","tool_call_id":"call_1","content":"second"}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"toolResult":{"content":[{"text":"first"}],"toolUseId":"call_1"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_tool_message_without_an_id_uses_the_generated_one()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"tool","content":"x"}]"#);
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"toolResult":{"content":[{"text":"x"}],"toolUseId":"call_generated"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_tool_message_of_another_shape_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"tool","tool_call_id":"call_1","content":5}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::ToolMessageShape { message: 0, .. }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_tool_result_file_without_media_is_reported() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages =
        list(r#"[{"role":"tool","tool_call_id":"call_1","content":[{"type":"file","file":{}}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(error, ProcessorError::ToolResult { .. }));
}
