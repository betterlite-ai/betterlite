use crate::bedrock::error::PromptError;
use crate::bedrock_processor::error::ProcessorError;

use super::prelude::{Harness, Settings, list, messages};

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_conversation_becomes_text_turns() {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"hello"}]"#),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[{"text":"hi"}]},
                {"role":"assistant","content":[{"text":"hello"}]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_consecutive_user_messages_merge_into_one_turn()
 {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(r#"[{"role":"user","content":"a"},{"role":"user","content":"b"}]"#),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(r#"[{"role":"user","content":[{"text":"a"},{"text":"b"}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_blank_assistant_string_adds_no_block() {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"   "}]"#),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(r#"[{"role":"user","content":[{"text":"hi"}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_tool_result_keeps_its_text_and_image() {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(
                r#"[{"role":"user","content":"weather?"},
                    {"role":"assistant","content":null,"tool_calls":[
                        {"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}}
                    ]},
                    {"role":"tool","tool_call_id":"call_1","content":[
                        {"type":"text","text":"sunny"},
                        {"type":"image_url","image_url":{"url":"data:image/png;base64,QUJD"}}
                    ]}]"#,
            ),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[{"text":"weather?"}]},
                {"role":"assistant","content":[{"toolUse":{
                    "input":{"city":"Paris"},"name":"get_weather","toolUseId":"call_1"
                }}]},
                {"role":"user","content":[{"toolResult":{"content":[
                    {"text":"sunny"},
                    {"image":{"format":"png","source":{"bytes":"QUJD"}}}
                ],"toolUseId":"call_1"}}]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_tool_message_without_an_id_takes_the_generated_one()
 {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(r#"[{"role":"tool","content":"windy"}]"#),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[{"toolResult":{
                "content":[{"text":"windy"}],"toolUseId":"generated-id"
            }}]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_an_unsupported_role_names_its_index() {
    let harness = Harness::new();
    let error = harness
        .convert(
            &messages(r#"[{"role":"user","content":"hi"},{"role":"system","content":"be brief"}]"#),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PromptError::Processor(ProcessorError::UnsupportedRole { message: 1, .. })
    ));
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_duplicate_tool_result_keeps_the_first_block()
 {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(
                r#"[{"role":"user","content":"go"},
                    {"role":"assistant","content":null,"tool_calls":[
                        {"id":"dup","type":"function","function":{"name":"first","arguments":"{}"}}
                    ]},
                    {"role":"tool","tool_call_id":"dup","content":"one"},
                    {"role":"tool","tool_call_id":"dup","content":"two"}]"#,
            ),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[{"text":"go"}]},
                {"role":"assistant","content":[{"toolUse":{
                    "input":{},"name":"first","toolUseId":"dup"
                }}]},
                {"role":"user","content":[{"toolResult":{
                    "content":[{"text":"one"}],"toolUseId":"dup"
                }}]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_duplicate_tool_use_keeps_the_first_block()
{
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(
                r#"[{"role":"assistant","content":null,"tool_calls":[
                    {"id":"dup","type":"function","function":{"name":"first","arguments":"{}"}},
                    {"id":"dup","type":"function","function":{"name":"second","arguments":"{}"}}
                ]}]"#,
            ),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"assistant","content":[{"toolUse":{
                "input":{},"name":"first","toolUseId":"dup"
            }}]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_duplicate_document_name_gets_a_positional_suffix()
 {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(
                r#"[{"role":"user","content":[{"type":"document","source":{
                        "type":"base64","media_type":"application/pdf","data":"QUJD"}}]},
                    {"role":"assistant","content":"ok"},
                    {"role":"user","content":[{"type":"document","source":{
                        "type":"base64","media_type":"application/pdf","data":"QUJD"}}]}]"#,
            ),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[
                    {"document":{"format":"pdf","name":"Document_0a702913308f6ec6_pdf",
                        "source":{"bytes":"QUJD"}}},
                    {"text":"."}]},
                {"role":"assistant","content":[{"text":"ok"}]},
                {"role":"user","content":[
                    {"document":{"format":"pdf","name":"Document_0a702913308f6ec6_pdf_2",
                        "source":{"bytes":"QUJD"}}},
                    {"text":"."}]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_document_only_turn_gets_a_placeholder_text()
 {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(
                r#"[{"role":"user","content":[{"type":"document","source":{
                    "type":"base64","media_type":"application/pdf","data":"QUJD"}}]}]"#,
            ),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[
                {"document":{"format":"pdf","name":"Document_0a702913308f6ec6_pdf",
                    "source":{"bytes":"QUJD"}}},
                {"text":"."}]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_thinking_comes_before_text_and_tool_use() {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(
                r#"[{"role":"user","content":"solve"},
                    {"role":"assistant","content":[
                        {"type":"text","text":"answer"},
                        {"type":"thinking","thinking":"why","signature":"sig"}
                    ],"tool_calls":[
                        {"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}
                    ]}]"#,
            ),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[{"text":"solve"}]},
                {"role":"assistant","content":[
                    {"reasoningContent":{"reasoningText":{"signature":"sig","text":"why"}}},
                    {"text":"answer"},
                    {"toolUse":{"input":{},"name":"f","toolUseId":"call_1"}}
                ]}]"#
        )
    );
}
