use crate::bedrock::error::PromptError;
use crate::bedrock_images::error::BedrockImageError;
use crate::bedrock_processor::error::ProcessorError;
use crate::bedrock_tool_results::error::BedrockToolResultsError;
use crate::bedrock_tools::error::BedrockToolError;

use super::prelude::{Harness, Settings, list, messages};

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_an_unsupported_image_format_fails() {
    let harness = Harness::new();
    let error = harness
        .convert(
            &messages(
                r#"[{"role":"user","content":[
                    {"type":"image_url","image_url":{"url":"data:image/tiff;base64,AAAA"}}
                ]}]"#,
            ),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PromptError::Processor(ProcessorError::Image {
            block: 0,
            source: BedrockImageError::UnsupportedImageFormat { .. }
        })
    ));
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_an_image_url_fails_through_the_injected_fetch()
 {
    let harness = Harness::new();
    let error = harness
        .convert(
            &messages(
                r#"[{"role":"user","content":[
                    {"type":"image_url","image_url":{"url":"https://pictures.test/a.png"}}
                ]}]"#,
            ),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PromptError::Processor(ProcessorError::Image {
            block: 0,
            source: BedrockImageError::Url(_)
        })
    ));
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_document_source_that_is_not_base64_fails()
{
    let harness = Harness::new();
    let error = harness
        .convert(
            &messages(
                r#"[{"role":"user","content":[{"type":"document","source":{
                    "type":"url","url":"https://files.test/a.pdf"
                }}]}]"#,
            ),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PromptError::Processor(ProcessorError::DocumentSourceNotBase64 { block: 0 })
    ));
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_tool_result_file_without_media_fails() {
    let harness = Harness::new();
    let error = harness
        .convert(
            &messages(
                r#"[{"role":"tool","tool_call_id":"call_1","content":[
                    {"type":"file","file":{}}
                ]}]"#,
            ),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PromptError::Processor(ProcessorError::ToolResult {
            source: BedrockToolResultsError::FileWithoutMedia { index: 0 }
        })
    ));
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_tool_call_without_an_id_fails() {
    let harness = Harness::new();
    let error = harness
        .convert(
            &messages(
                r#"[{"role":"assistant","content":null,"tool_calls":[
                    {"type":"function","function":{"name":"f","arguments":"{}"}}
                ]}]"#,
            ),
            &[],
            &Settings::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PromptError::Processor(ProcessorError::ToolInvoke {
            source: BedrockToolResultsError::ToolCallWithoutId { index: 0 }
        })
    ));
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_tool_that_is_not_an_object_fails() {
    let harness = Harness::new();
    let error = harness
        .convert(
            &messages(r#"[{"role":"user","content":"go"}]"#),
            &list(r#"["not a tool"]"#),
            &Settings::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PromptError::Tools(BedrockToolError::ToolNotObject { index: 0 })
    ));
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_malformed_function_tool_fails() {
    let harness = Harness::new();
    let error = harness
        .convert(
            &messages(r#"[{"role":"user","content":"go"}]"#),
            &list(r#"[{"type":"function","function":"get_weather"}]"#),
            &Settings::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PromptError::Tools(BedrockToolError::WrongType { field: "function" })
    ));
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_native_tool_block_passes_through_unchecked()
 {
    let harness = Harness::new();
    let tools = list(r#"[{"toolSpec": null}, {"cachePoint": {"type": "default"}}]"#);
    let converted = harness
        .convert(
            &messages(r#"[{"role":"user","content":"go"}]"#),
            &tools,
            &Settings::default(),
        )
        .await
        .unwrap();
    assert_eq!(converted.tools, tools);
}
