use std::collections::BTreeMap;

use crate::anthropic_results::tool_result::ToolResultError;
use crate::anthropic_user_blocks::error::UserBlocksError;
use crate::anthropic_user_blocks::user_run::{UserBlockBuilder, UserRun};

use super::{NoFileBlocks, RecordingBlocks, UrlMap, messages, object, url_map};

const GENERATED_ID: &str = "generated-id";

async fn convert_with(
    raw: &str,
    force_base64: bool,
    urls: &BTreeMap<String, String>,
) -> Result<UserRun, UserBlocksError<NoFileBlocks>> {
    let input = messages(raw);
    let fetcher = UrlMap {
        data_urls: urls.clone(),
    };
    let blocks = NoFileBlocks;
    let new_id = || GENERATED_ID.to_owned();
    let builder = UserBlockBuilder {
        force_base64,
        fetcher: &fetcher,
        blocks: &blocks,
        new_id: &new_id,
    };
    builder.convert(&input, 0).await
}

async fn run(raw: &str) -> UserRun {
    convert_with(raw, false, &BTreeMap::new()).await.unwrap()
}

async fn failure(raw: &str) -> UserBlocksError<NoFileBlocks> {
    convert_with(raw, false, &BTreeMap::new())
        .await
        .unwrap_err()
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_user_string_content_becomes_a_text_block()
{
    let run = run(r#"[{"role":"user","content":"hi"}]"#).await;
    assert_eq!(run.blocks, vec![object(r#"{"type":"text","text":"hi"}"#)]);
    assert_eq!(run.next, 1);
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_message_cache_control_reaches_its_text_block()
 {
    let run = run(r#"[{"role":"user","content":"hi","cache_control":{"type":"ephemeral"}}]"#).await;
    assert_eq!(
        run.blocks,
        vec![object(
            r#"{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}"#
        )]
    );
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_non_object_cache_control_is_ignored() {
    let run = run(r#"[{"role":"user","content":"hi","cache_control":"ephemeral"}]"#).await;
    assert_eq!(run.blocks, vec![object(r#"{"type":"text","text":"hi"}"#)]);
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_consecutive_user_messages_merge_into_one_turn()
 {
    let run = run(r#"[{"role":"user","content":"a"},{"role":"user","content":"b"}]"#).await;
    assert_eq!(
        run.blocks,
        vec![
            object(r#"{"type":"text","text":"a"}"#),
            object(r#"{"type":"text","text":"b"}"#)
        ]
    );
    assert_eq!(run.next, 2);
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_text_block_keeps_its_cache_control() {
    let run = run(
        r#"[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral","ttl":"5m"}}]}]"#,
    )
    .await;
    assert_eq!(
        run.blocks,
        vec![object(
            r#"{"type":"text","text":"a","cache_control":{"type":"ephemeral","ttl":"5m"}}"#
        )]
    );
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_an_image_url_string_becomes_a_base64_image()
{
    let run = run(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":"data:image/png;base64,QUJD"}]}]"#,
    )
    .await;
    assert_eq!(
        run.blocks,
        vec![object(
            r#"{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}"#
        )]
    );
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_an_image_url_object_sets_the_media_type() {
    let run = run(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,QUJD","format":"image/webp"},"cache_control":{"type":"ephemeral"}}]}]"#,
    )
    .await;
    assert_eq!(
        run.blocks,
        vec![object(
            r#"{"type":"image","source":{"type":"base64","media_type":"image/webp","data":"QUJD"},"cache_control":{"type":"ephemeral"}}"#
        )]
    );
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_force_base64_downloads_an_https_image() {
    let urls = url_map(&[("https://example.test/a.png", "data:image/png;base64,QUJD")]);
    let run = convert_with(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/a.png"}}]}]"#,
        true,
        &urls,
    )
    .await
    .unwrap();
    assert_eq!(
        run.blocks,
        vec![object(
            r#"{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}"#
        )]
    );
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_an_https_image_stays_a_url_source() {
    let run = run(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/a.png"}}]}]"#,
    )
    .await;
    assert_eq!(
        run.blocks,
        vec![object(
            r#"{"type":"image","source":{"type":"url","url":"https://example.test/a.png"}}"#
        )]
    );
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_document_block_is_copied_through() {
    let run = run(
        r#"[{"role":"user","content":[{"type":"document","source":{"type":"text","media_type":"text/plain","data":"aGk="},"title":"notes","cache_control":{"type":"ephemeral"}}]}]"#,
    )
    .await;
    assert_eq!(
        run.blocks,
        vec![object(
            r#"{"type":"document","source":{"type":"text","media_type":"text/plain","data":"aGk="},"title":"notes","cache_control":{"type":"ephemeral"}}"#
        )]
    );
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_file_block_becomes_a_document_block() {
    let run = run(
        r#"[{"role":"user","content":[{"type":"file","file":{"file_data":"data:application/pdf;base64,QUJD","format":"application/pdf"},"cache_control":{"type":"ephemeral"}}]}]"#,
    )
    .await;
    assert_eq!(
        run.blocks,
        vec![object(
            r#"{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"QUJD"},"cache_control":{"type":"ephemeral"}}"#
        )]
    );
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_an_unknown_block_type_is_omitted() {
    let run = run(
        r#"[{"role":"user","content":[{"type":"audio","data":"x"},{"type":"text","text":"keep"}]}]"#,
    )
    .await;
    assert_eq!(run.blocks, vec![object(r#"{"type":"text","text":"keep"}"#)]);
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_null_content_contributes_no_block() {
    let run = run(r#"[{"role":"user","content":null}]"#).await;
    assert!(run.blocks.is_empty());
    assert_eq!(run.next, 1);
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_an_empty_content_list_contributes_no_block()
{
    let run = run(r#"[{"role":"user","content":[]}]"#).await;
    assert!(run.blocks.is_empty());
    assert_eq!(run.next, 1);
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_tool_message_becomes_a_tool_result() {
    let run = run(r#"[{"role":"tool","tool_call_id":"call.123","content":"hi"}]"#).await;
    assert_eq!(
        run.blocks,
        vec![object(
            r#"{"type":"tool_result","tool_use_id":"call_123","content":"hi"}"#
        )]
    );
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_function_message_becomes_a_tool_result() {
    let run =
        run(r#"[{"role":"function","name":"weather","tool_call_id":"call.1","content":"hi"}]"#)
            .await;
    assert_eq!(
        run.blocks,
        vec![object(
            r#"{"type":"tool_result","tool_use_id":"call_1","content":"hi"}"#
        )]
    );
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_tool_result_file_reaches_the_policy() {
    let input = messages(
        r#"[{"role":"tool","tool_call_id":"call","content":[{"type":"image_url","image_url":{"url":"data:application/pdf;base64,QQ=="}}]}]"#,
    );
    let fetcher = UrlMap {
        data_urls: BTreeMap::new(),
    };
    let blocks = RecordingBlocks;
    let new_id = || GENERATED_ID.to_owned();
    let builder = UserBlockBuilder {
        force_base64: false,
        fetcher: &fetcher,
        blocks: &blocks,
        new_id: &new_id,
    };
    let run = builder.convert(&input, 0).await.unwrap();
    assert_eq!(
        run.blocks,
        vec![object(
            r#"{"file_block":{"type":"file","file":{"file_data":"data:application/pdf;base64,QQ=="}}}"#
        )]
    );
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_tool_message_without_an_id_is_a_typed_failure()
 {
    let error = failure(r#"[{"role":"tool","content":"hi"}]"#).await;
    assert!(matches!(
        error,
        UserBlocksError::ToolResult(ToolResultError::MissingToolCallId)
    ));
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_text_block_without_text_is_a_typed_failure()
 {
    let error = failure(r#"[{"role":"user","content":[{"type":"text"}]}]"#).await;
    assert!(matches!(
        error,
        UserBlocksError::Field { field: "text", .. }
    ));
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_an_image_block_without_a_url_is_a_typed_failure()
 {
    let error = failure(r#"[{"role":"user","content":[{"type":"image_url"}]}]"#).await;
    assert!(matches!(
        error,
        UserBlocksError::Field {
            field: "image_url",
            ..
        }
    ));
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_non_object_block_is_a_typed_failure() {
    let error = failure(r#"[{"role":"user","content":[5]}]"#).await;
    assert!(matches!(
        error,
        UserBlocksError::BlockNotObject {
            message: 0,
            block: 0
        }
    ));
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_message_without_a_role_is_a_typed_failure()
{
    let error = failure(r#"[{"content":"hi"}]"#).await;
    assert!(matches!(error, UserBlocksError::MissingRole { index: 0 }));
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_an_assistant_message_ends_the_user_run() {
    let run = run(r#"[{"role":"assistant","content":"hi"}]"#).await;
    assert!(run.blocks.is_empty());
    assert_eq!(run.next, 0);
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_the_run_stops_at_the_first_assistant_message()
 {
    let input = messages(
        r#"[{"role":"user","content":"a"},{"role":"assistant","content":"b"},{"role":"user","content":"c"}]"#,
    );
    let fetcher = UrlMap {
        data_urls: BTreeMap::new(),
    };
    let blocks = NoFileBlocks;
    let new_id = || GENERATED_ID.to_owned();
    let builder = UserBlockBuilder {
        force_base64: false,
        fetcher: &fetcher,
        blocks: &blocks,
        new_id: &new_id,
    };
    let run = builder.convert(&input, 0).await.unwrap();
    assert_eq!(run.blocks, vec![object(r#"{"type":"text","text":"a"}"#)]);
    assert_eq!(run.next, 1);
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_a_run_starting_past_the_end_is_empty() {
    let input = messages(r#"[{"role":"user","content":"a"}]"#);
    let fetcher = UrlMap {
        data_urls: BTreeMap::new(),
    };
    let blocks = NoFileBlocks;
    let new_id = || GENERATED_ID.to_owned();
    let builder = UserBlockBuilder {
        force_base64: false,
        fetcher: &fetcher,
        blocks: &blocks,
        new_id: &new_id,
    };
    let run = builder.convert(&input, 1).await.unwrap();
    assert!(run.blocks.is_empty());
    assert_eq!(run.next, 1);
}

#[tokio::test]
async fn anthropic_user_blocks_behavior_tests_behavior_the_source_messages_are_unchanged() {
    let input = messages(
        r#"[{"role":"user","content":[{"type":"text","text":"a"},{"type":"image_url","image_url":"data:image/png;base64,QUJD"}]}]"#,
    );
    let original = input.clone();
    let fetcher = UrlMap {
        data_urls: BTreeMap::new(),
    };
    let blocks = NoFileBlocks;
    let new_id = || GENERATED_ID.to_owned();
    let builder = UserBlockBuilder {
        force_base64: false,
        fetcher: &fetcher,
        blocks: &blocks,
        new_id: &new_id,
    };
    builder.convert(&input, 0).await.unwrap();
    assert_eq!(input, original);
}
