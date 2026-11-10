use std::sync::atomic::{AtomicU32, Ordering};

use rstest::rstest;

use crate::bedrock_tool_results::error::BedrockToolResultsError;
use crate::bedrock_tool_results::result::to_bedrock_tool_result;

use super::prelude::{convert_result, expected, field, message, remote};

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_text_content_becomes_one_text_block() {
    assert_eq!(
        convert_result(
            r#"{"role":"tool","tool_call_id":"call_1","content":"sunny"}"#,
            "unused"
        )
        .await
        .unwrap(),
        expected(r#"{"toolResult":{"content":[{"text":"sunny"}],"toolUseId":"call_1"}}"#)
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_an_empty_text_content_is_kept() {
    assert_eq!(
        convert_result(
            r#"{"role":"tool","tool_call_id":"call_1","content":""}"#,
            "unused"
        )
        .await
        .unwrap(),
        expected(r#"{"toolResult":{"content":[{"text":""}],"toolUseId":"call_1"}}"#)
    );
}

#[rstest]
#[case::absent(r#"{"role":"tool","tool_call_id":"call_1"}"#)]
#[case::null(r#"{"role":"tool","tool_call_id":"call_1","content":null}"#)]
#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_missing_content_becomes_no_blocks(
    #[case] raw_message: &str,
) {
    assert_eq!(
        convert_result(raw_message, "unused").await.unwrap(),
        expected(r#"{"toolResult":{"content":[],"toolUseId":"call_1"}}"#)
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_an_empty_content_list_becomes_no_blocks() {
    assert_eq!(
        convert_result(
            r#"{"role":"tool","tool_call_id":"call_1","content":[]}"#,
            "unused"
        )
        .await
        .unwrap(),
        expected(r#"{"toolResult":{"content":[],"toolUseId":"call_1"}}"#)
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_text_parts_keep_their_order() {
    assert_eq!(
        convert_result(
            r#"{"role":"tool","tool_call_id":"call_1","content":[{"type":"text","text":"one "},{"type":"text","text":"two"}]}"#,
            "unused"
        )
        .await
        .unwrap(),
        expected(
            r#"{"toolResult":{"content":[{"text":"one "},{"text":"two"}],"toolUseId":"call_1"}}"#
        )
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_unknown_content_parts_are_skipped() {
    assert_eq!(
        convert_result(
            r#"{"role":"tool","tool_call_id":"call_1","content":[{"type":"tool_reference","tool_name":"x"},{"type":"text","text":"kept"}]}"#,
            "unused"
        )
        .await
        .unwrap(),
        expected(r#"{"toolResult":{"content":[{"text":"kept"}],"toolUseId":"call_1"}}"#)
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_search_results_replace_the_content() {
    assert_eq!(
        convert_result(
            r#"{"role":"tool","tool_call_id":"call_1","content":"ignored","search_results":[{"source":"s","title":"t","content":[{"text":"c"}],"citations":{"x":1}}]}"#,
            "unused"
        )
        .await
        .unwrap(),
        expected(
            r#"{"toolResult":{"content":[{"searchResult":{"source":"s","title":"t","content":[{"text":"c"}],"citations":{"x":1}}}],"toolUseId":"call_1","status":"success"}}"#
        )
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_non_object_search_results_are_dropped() {
    assert_eq!(
        convert_result(
            r#"{"role":"tool","tool_call_id":"call_1","search_results":[5,"str",{"source":"s"}]}"#,
            "unused"
        )
        .await
        .unwrap(),
        expected(
            r#"{"toolResult":{"content":[{"searchResult":{"source":"s"}}],"toolUseId":"call_1","status":"success"}}"#
        )
    );
}

#[rstest]
#[case::all_non_objects(r#"[5,"str"]"#)]
#[case::empty("[]")]
#[case::null("null")]
#[case::not_a_list(r#""nope""#)]
#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_unusable_search_results_fall_back_to_content(
    #[case] search_results: &str,
) {
    let raw_message = format!(
        r#"{{"role":"tool","tool_call_id":"call_1","content":"kept","search_results":{search_results}}}"#
    );
    assert_eq!(
        convert_result(&raw_message, "unused").await.unwrap(),
        expected(r#"{"toolResult":{"content":[{"text":"kept"}],"toolUseId":"call_1"}}"#)
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_the_tool_call_id_is_sanitized() {
    let block = convert_result(
        r#"{"role":"tool","tool_call_id":"call/1","content":"x"}"#,
        "unused",
    )
    .await
    .unwrap();
    let tool_use_id = field(field(&block, "toolResult"), "toolUseId")
        .as_str()
        .unwrap();
    assert!(tool_use_id.starts_with("call_1_"), "{tool_use_id}");
    assert_eq!(tool_use_id.len(), 15);
}

#[rstest]
#[case::absent(r#"{"role":"tool","content":"x"}"#)]
#[case::null(r#"{"role":"tool","tool_call_id":null,"content":"x"}"#)]
#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_a_missing_tool_call_id_uses_the_new_id(
    #[case] raw_message: &str,
) {
    assert_eq!(
        convert_result(raw_message, "generated-id").await.unwrap(),
        expected(r#"{"toolResult":{"content":[{"text":"x"}],"toolUseId":"generated-id"}}"#)
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_a_non_string_tool_call_id_is_refused() {
    let error = convert_result(
        r#"{"role":"tool","tool_call_id":5,"content":"x"}"#,
        "unused",
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        BedrockToolResultsError::ResultToolCallIdNotText
    ));
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_the_new_id_is_read_only_when_it_is_needed() {
    let remote = remote();
    let reads = AtomicU32::new(0);
    let ids = || {
        reads.fetch_add(1, Ordering::Relaxed);
        "generated-id".to_owned()
    };
    let present = message(r#"{"role":"tool","tool_call_id":"call_1","content":"x"}"#);
    drop(
        to_bedrock_tool_result(&present, &remote, &ids)
            .await
            .unwrap(),
    );
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    let missing = message(r#"{"role":"tool","content":"x"}"#);
    let block = to_bedrock_tool_result(&missing, &remote, &ids)
        .await
        .unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    assert_eq!(block.tool_result.tool_use_id, "generated-id");
}

#[rstest]
#[case::missing_text(r#"{"type":"text"}"#, "TextPartWithoutText")]
#[case::null_text(r#"{"type":"text","text":null}"#, "TextPartNotText")]
#[case::numeric_text(r#"{"type":"text","text":5}"#, "TextPartNotText")]
#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_an_unusable_text_part_names_its_failure(
    #[case] part: &str,
    #[case] variant: &str,
) {
    let raw_message = format!(r#"{{"role":"tool","tool_call_id":"call_1","content":[{part}]}}"#);
    let error = convert_result(&raw_message, "unused").await.unwrap_err();
    assert_eq!(format!("{error:?}").split([' ', '{']).next(), Some(variant));
}
