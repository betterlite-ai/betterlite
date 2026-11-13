use std::collections::BTreeMap;

use crate::anthropic_results::tool_result::{ToolResultError, to_anthropic_tool_result};
use crate::arguments::json::{Json, Object};
use crate::media::limits::MAX_MESSAGE_PARTS;
use crate::message_content::{Content, ContentPart, Message};

use super::{FailingBlocks, MapSource, NoFileBlocks, RecordingBlocks, maps, message, object};

const GENERATED_ID: &str = "generated-id";

async fn convert_with(raw: &str, force_base64: bool, urls: &BTreeMap<String, String>) -> Object {
    let fetcher = MapSource { data_urls: urls };
    let blocks = RecordingBlocks;
    to_anthropic_tool_result(&message(raw), force_base64, &fetcher, &blocks, &|| {
        GENERATED_ID.to_owned()
    })
    .await
    .unwrap()
}

async fn convert(raw: &str, force_base64: bool) -> Object {
    let urls = BTreeMap::new();
    convert_with(raw, force_base64, &urls).await
}

async fn convert_error(raw: &str) -> ToolResultError<NoFileBlocks> {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let blocks = RecordingBlocks;
    to_anthropic_tool_result(&message(raw), false, &fetcher, &blocks, &|| {
        GENERATED_ID.to_owned()
    })
    .await
    .expect_err("the conversion must fail")
}

fn message_with_parts(count: usize) -> Message {
    let parts = (0..count)
        .map(|index| ContentPart {
            kind: "text".to_owned(),
            fields: Object::from_iter([("text".to_owned(), Json::Text(index.to_string()))]),
        })
        .collect();
    Message {
        role: "tool".to_owned(),
        content: Some(Content::Parts(parts)),
        name: None,
        provider_specific_fields: Object::from_iter([(
            "tool_call_id".to_owned(),
            Json::Text("call".to_owned()),
        )]),
    }
}

fn expected_tool_result(count: usize) -> Object {
    let content = (0..count)
        .map(|index| {
            Json::Object(Object::from_iter([
                ("type".to_owned(), Json::Text("text".to_owned())),
                ("text".to_owned(), Json::Text(index.to_string())),
            ]))
        })
        .collect();
    Object::from_iter([
        ("type".to_owned(), Json::Text("tool_result".to_owned())),
        ("tool_use_id".to_owned(), Json::Text("call".to_owned())),
        ("content".to_owned(), Json::List(content)),
    ])
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_tool_message_becomes_a_tool_result() {
    let input = message(r#"{"role":"tool","tool_call_id":"call.123","content":"hi"}"#);
    let original = input.clone();
    let urls = BTreeMap::new();
    let block = convert_with_message(&input, &urls).await;
    assert_eq!(
        block,
        object(r#"{"type":"tool_result","tool_use_id":"call_123","content":"hi"}"#)
    );
    assert_eq!(input, original);
}

async fn convert_with_message(input: &Message, urls: &BTreeMap<String, String>) -> Object {
    let fetcher = MapSource { data_urls: urls };
    let blocks = RecordingBlocks;
    to_anthropic_tool_result(input, false, &fetcher, &blocks, &|| GENERATED_ID.to_owned())
        .await
        .unwrap()
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_tool_message_without_an_id_is_an_error() {
    let error = convert_error(r#"{"role":"tool","content":"hi"}"#).await;
    assert!(matches!(error, ToolResultError::MissingToolCallId));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_non_string_tool_call_id_is_named() {
    let error = convert_error(r#"{"role":"tool","tool_call_id":7,"content":"hi"}"#).await;
    assert!(matches!(
        error,
        ToolResultError::Field {
            field: "tool_call_id"
        }
    ));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_function_message_generates_a_missing_id() {
    let block = convert(
        r#"{"role":"function","name":"weather","content":"hi"}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(r#"{"type":"tool_result","tool_use_id":"generated-id","content":"hi"}"#)
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_function_message_keeps_its_id() {
    let block = convert(
        r#"{"role":"function","name":"weather","tool_call_id":"call.1","content":"hi"}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(r#"{"type":"tool_result","tool_use_id":"call_1","content":"hi"}"#)
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_an_explicit_empty_function_id_is_used_as_given()
{
    let block = convert(
        r#"{"role":"function","name":"weather","tool_call_id":"","content":"hi"}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(r#"{"type":"tool_result","tool_use_id":"tool_use_id","content":"hi"}"#)
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_another_role_is_an_error() {
    let error = convert_error(r#"{"role":"user","content":"hi"}"#).await;
    assert!(matches!(error, ToolResultError::Role { .. }));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_null_content_becomes_an_empty_string() {
    let block = convert(
        r#"{"role":"tool","tool_call_id":"call","content":null}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(r#"{"type":"tool_result","tool_use_id":"call","content":""}"#)
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_an_empty_list_stays_an_empty_list() {
    let block = convert(
        r#"{"role":"tool","tool_call_id":"call","content":[]}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(r#"{"type":"tool_result","tool_use_id":"call","content":[]}"#)
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_text_part_copies_its_cache_control() {
    let block = convert(
        r#"{"role":"tool","tool_call_id":"call","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(
            r#"{"type":"tool_result","tool_use_id":"call","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_text_part_copies_a_non_object_cache_control() {
    let block = convert(
        r#"{"role":"tool","tool_call_id":"call","content":[{"type":"text","text":"a","cache_control":"ephemeral"}]}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(
            r#"{"type":"tool_result","tool_use_id":"call","content":[{"type":"text","text":"a","cache_control":"ephemeral"}]}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_text_part_without_cache_control_has_none() {
    let block = convert(
        r#"{"role":"tool","tool_call_id":"call","content":[{"type":"text","text":"a"}]}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(
            r#"{"type":"tool_result","tool_use_id":"call","content":[{"type":"text","text":"a"}]}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_text_part_without_text_is_an_error() {
    let error =
        convert_error(r#"{"role":"tool","tool_call_id":"call","content":[{"type":"text"}]}"#).await;
    assert!(matches!(error, ToolResultError::Field { field: "text" }));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_string_image_url_becomes_an_image_block() {
    let block = convert(
        r#"{"role":"tool","tool_call_id":"call","content":[{"type":"image_url","image_url":"data:image/png;base64,QUJD"}]}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(
            r#"{"type":"tool_result","tool_use_id":"call","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}]}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_an_object_image_url_sets_the_format() {
    let block = convert(
        r#"{"role":"tool","tool_call_id":"call","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,QUJD","format":"image/webp"}}]}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(
            r#"{"type":"tool_result","tool_use_id":"call","content":[{"type":"image","source":{"type":"base64","media_type":"image/webp","data":"QUJD"}}]}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_an_unknown_part_type_is_omitted() {
    let block = convert(
        r#"{"role":"tool","tool_call_id":"call","content":[{"type":"audio","data":"x"},{"type":"text","text":"keep"}]}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(
            r#"{"type":"tool_result","tool_use_id":"call","content":[{"type":"text","text":"keep"}]}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_tool_reference_is_copied() {
    let block = convert(
        r#"{"role":"tool","tool_call_id":"call","content":[{"type":"tool_reference","tool_name":"lookup"}]}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(
            r#"{"type":"tool_result","tool_use_id":"call","content":[{"type":"tool_reference","tool_name":"lookup"}]}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_tool_reference_without_a_name_is_an_error() {
    let error = convert_error(
        r#"{"role":"tool","tool_call_id":"call","content":[{"type":"tool_reference"}]}"#,
    )
    .await;
    assert!(matches!(
        error,
        ToolResultError::Field { field: "tool_name" }
    ));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_file_part_reaches_the_file_block_policy() {
    let block = convert(
        r#"{"role":"tool","tool_call_id":"call","content":[{"type":"file","file":{"file_data":"data:application/pdf;base64,QQ=="}}]}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(
            r#"{"type":"tool_result","tool_use_id":"call","content":[{"file_block":{"type":"file","file":{"file_data":"data:application/pdf;base64,QQ=="}}}]}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_document_data_uri_becomes_a_file_data_element()
{
    let block = convert(
        r#"{"role":"tool","tool_call_id":"call","content":[{"type":"image_url","image_url":{"url":"data:application/pdf;base64,QQ=="},"cache_control":{"type":"ephemeral"}}]}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(
            r#"{"type":"tool_result","tool_use_id":"call","content":[{"file_block":{"type":"file","file":{"file_data":"data:application/pdf;base64,QQ=="}},"cache_control":{"type":"ephemeral"}}]}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_file_blocks_failure_is_returned() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let input = message(
        r#"{"role":"tool","tool_call_id":"call","content":[{"type":"file","file":{"file_data":"data:application/pdf;base64,QQ=="}}]}"#,
    );
    let error = to_anthropic_tool_result(&input, false, &fetcher, &FailingBlocks, &|| {
        GENERATED_ID.to_owned()
    })
    .await
    .expect_err("the conversion must fail");
    assert!(matches!(error, ToolResultError::File(_)));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_the_message_cache_control_is_added() {
    let block = convert(
        r#"{"role":"tool","tool_call_id":"call","content":"hi","cache_control":{"type":"ephemeral"}}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(
            r#"{"type":"tool_result","tool_use_id":"call","content":"hi","cache_control":{"type":"ephemeral"}}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_null_message_cache_control_is_ignored() {
    let block = convert(
        r#"{"role":"tool","tool_call_id":"call","content":"hi","cache_control":null}"#,
        false,
    )
    .await;
    assert_eq!(
        block,
        object(r#"{"type":"tool_result","tool_use_id":"call","content":"hi"}"#)
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_an_image_part_without_image_url_is_an_error() {
    let error =
        convert_error(r#"{"role":"tool","tool_call_id":"call","content":[{"type":"image_url"}]}"#)
            .await;
    assert!(matches!(error, ToolResultError::Image(_)));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_force_base64_downloads_an_https_image() {
    let urls = maps(&[("https://example.test/a.png", "data:image/png;base64,QUJD")]);
    let block = convert_with(
        r#"{"role":"tool","tool_call_id":"call","content":[{"type":"image_url","image_url":{"url":"https://example.test/a.png"}}]}"#,
        true,
        &urls,
    )
    .await;
    assert_eq!(
        block,
        object(
            r#"{"type":"tool_result","tool_use_id":"call","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}]}"#
        )
    );
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_a_full_parts_list_still_converts() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let block = to_anthropic_tool_result(
        &message_with_parts(MAX_MESSAGE_PARTS),
        false,
        &fetcher,
        &RecordingBlocks,
        &|| GENERATED_ID.to_owned(),
    )
    .await
    .unwrap();
    assert_eq!(block, expected_tool_result(MAX_MESSAGE_PARTS));
}

#[tokio::test]
async fn anthropic_results_behavior_tests_behavior_too_many_parts_are_refused() {
    let urls = BTreeMap::new();
    let fetcher = MapSource { data_urls: &urls };
    let error = to_anthropic_tool_result(
        &message_with_parts(MAX_MESSAGE_PARTS + 1),
        false,
        &fetcher,
        &RecordingBlocks,
        &|| GENERATED_ID.to_owned(),
    )
    .await
    .expect_err("the conversion must fail");
    assert!(matches!(error, ToolResultError::TooManyParts { .. }));
}
