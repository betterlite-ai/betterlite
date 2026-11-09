use crate::arguments::json::Json;
use crate::bedrock_content::dedup::{
    TOOL_RESULT_BLOCK, TOOL_USE_BLOCK, TOOL_USE_ID, deduplicate_content_blocks,
};
use crate::bedrock_content::error::ContentError;

use super::prelude::{capture, json};

const ID_KEY: &str = TOOL_USE_ID;

#[test]
fn bedrock_content_behavior_tests_behavior_duplicate_tool_result_keeps_first() {
    let blocks = vec![
        json(r#"{"toolResult":{"toolUseId":"id-1","content":[{"text":"first"}]}}"#),
        json(r#"{"cachePoint":{"type":"default"}}"#),
        json(r#"{"toolResult":{"toolUseId":"id-1","content":[{"text":"second"}]}}"#),
        json(r#"{"toolResult":{"toolUseId":"id-2","content":[{"text":"third"}]}}"#),
    ];
    let deduplicated = deduplicate_content_blocks(&blocks, TOOL_RESULT_BLOCK, ID_KEY).unwrap();
    assert_eq!(
        deduplicated,
        [blocks[0].clone(), blocks[1].clone(), blocks[3].clone()]
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_tool_use_duplicates_are_deduplicated() {
    let blocks = vec![
        json(r#"{"toolUse":{"toolUseId":"tu-1","name":"a","input":{}}}"#),
        json(r#"{"text":"between"}"#),
        json(r#"{"toolUse":{"toolUseId":"tu-1","name":"a","input":{}}}"#),
        json(r#"{"toolUse":{"toolUseId":"tu-2","name":"b","input":{}}}"#),
    ];
    let deduplicated = deduplicate_content_blocks(&blocks, TOOL_USE_BLOCK, ID_KEY).unwrap();
    assert_eq!(
        deduplicated,
        [blocks[0].clone(), blocks[1].clone(), blocks[3].clone()]
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_blocks_without_the_key_are_preserved() {
    let blocks = vec![
        json(r#"{"text":"keep"}"#),
        json(r#"{"toolUse":{"toolUseId":"tu-1","name":"a","input":{}}}"#),
        json(r#"{"toolResult":null}"#),
        json(r#"{"toolResult":"not-an-object"}"#),
    ];
    let deduplicated = deduplicate_content_blocks(&blocks, TOOL_RESULT_BLOCK, ID_KEY).unwrap();
    assert_eq!(deduplicated, blocks);
}

#[rstest::rstest]
#[case::empty_ids(r#"[{"toolResult":{"toolUseId":""}},{"toolResult":{"toolUseId":""}}]"#)]
#[case::missing_ids(r#"[{"toolResult":{}},{"toolResult":{}}]"#)]
#[case::null_ids(r#"[{"toolResult":{"toolUseId":null}},{"toolResult":{"toolUseId":null}}]"#)]
fn bedrock_content_behavior_tests_behavior_blocks_without_a_usable_id_are_not_tracked(
    #[case] blocks_json: &str,
) {
    let blocks: Vec<Json> = serde_json::from_str(blocks_json).unwrap();
    let deduplicated = deduplicate_content_blocks(&blocks, TOOL_RESULT_BLOCK, ID_KEY).unwrap();
    assert_eq!(deduplicated, blocks);
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_custom_id_key_selects_the_identity() {
    let blocks = vec![
        json(r#"{"toolResult":{"other":"shared","content":[{"text":"first"}]}}"#),
        json(r#"{"toolResult":{"other":"shared","content":[{"text":"second"}]}}"#),
        json(r#"{"toolResult":{"other":"different","content":[{"text":"third"}]}}"#),
    ];
    let deduplicated = deduplicate_content_blocks(&blocks, TOOL_RESULT_BLOCK, "other").unwrap();
    assert_eq!(deduplicated, [blocks[0].clone(), blocks[2].clone()]);
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_custom_block_key_selects_the_identity() {
    let blocks = vec![
        json(r#"{"document":{"name":"shared"}}"#),
        json(r#"{"document":{"name":"shared"}}"#),
        json(r#"{"text":"keep"}"#),
    ];
    let deduplicated = deduplicate_content_blocks(&blocks, "document", "name").unwrap();
    assert_eq!(deduplicated, [blocks[0].clone(), blocks[2].clone()]);
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_non_object_block_is_rejected() {
    let blocks = vec![json(r#""text""#)];
    assert_eq!(
        deduplicate_content_blocks(&blocks, TOOL_RESULT_BLOCK, ID_KEY),
        Err(ContentError::BlockNotObject { index: 0 })
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_non_text_id_is_rejected() {
    let blocks = vec![json(r#"{"toolResult":{"toolUseId":7}}"#)];
    assert_eq!(
        deduplicate_content_blocks(&blocks, TOOL_RESULT_BLOCK, ID_KEY),
        Err(ContentError::BlockIdNotText {
            block_key: TOOL_RESULT_BLOCK.to_owned(),
        })
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_dropped_duplicate_emits_a_warning() {
    let (captured, _guard) = capture();
    let blocks = vec![
        json(r#"{"toolResult":{"toolUseId":"id-1"}}"#),
        json(r#"{"toolResult":{"toolUseId":"id-1"}}"#),
    ];
    deduplicate_content_blocks(&blocks, TOOL_RESULT_BLOCK, ID_KEY).unwrap();
    let text = captured.text();
    assert!(text.contains("duplicate"));
    assert!(text.contains("id-1"));
}
