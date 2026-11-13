use crate::bedrock_content::error::ContentError;
use crate::bedrock_content::order::sort_assistant_content_blocks;

use super::prelude::json;

#[test]
fn bedrock_content_behavior_tests_behavior_sort_orders_reasoning_then_content_then_tools() {
    let blocks = vec![
        json(r#"{"toolUse":{"toolUseId":"tu-1","name":"first","input":{}}}"#),
        json(r#"{"text":"answer"}"#),
        json(r#"{"reasoningContent":{"reasoningText":{"text":"because"}}}"#),
        json(r#"{"cachePoint":{"type":"default"}}"#),
        json(r#"{"image":{"format":"png"}}"#),
        json(r#"{"toolUse":{"toolUseId":"tu-2","name":"second","input":{}}}"#),
    ];
    let sorted = sort_assistant_content_blocks(&blocks).unwrap();
    assert_eq!(
        sorted,
        [
            blocks[2].clone(),
            blocks[1].clone(),
            blocks[4].clone(),
            blocks[0].clone(),
            blocks[3].clone(),
            blocks[5].clone(),
        ]
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_sort_keeps_equal_ranks_in_source_order() {
    let blocks = vec![
        json(r#"{"toolUse":{"toolUseId":"tu-1","name":"first","input":{}}}"#),
        json(r#"{"text":"first text"}"#),
        json(r#"{"toolUse":{"toolUseId":"tu-2","name":"second","input":{}}}"#),
        json(r#"{"text":"second text"}"#),
    ];
    let sorted = sort_assistant_content_blocks(&blocks).unwrap();
    assert_eq!(
        sorted,
        [
            blocks[1].clone(),
            blocks[3].clone(),
            blocks[0].clone(),
            blocks[2].clone(),
        ]
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_tool_use_and_its_cache_point_stay_together() {
    let blocks = vec![
        json(r#"{"toolUse":{"toolUseId":"tu-1","name":"first","input":{}}}"#),
        json(r#"{"cachePoint":{"type":"default"}}"#),
        json(r#"{"text":"answer"}"#),
    ];
    let sorted = sort_assistant_content_blocks(&blocks).unwrap();
    assert_eq!(
        sorted,
        [blocks[2].clone(), blocks[0].clone(), blocks[1].clone()]
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_block_with_reasoning_wins_over_tool_use() {
    let blocks = vec![
        json(r#"{"toolUse":{"toolUseId":"tu-1","name":"first","input":{}}}"#),
        json(
            r#"{"toolUse":{"toolUseId":"tu-2","name":"second","input":{}},"reasoningContent":{}}"#,
        ),
        json(r#"{"text":"answer"}"#),
    ];
    let sorted = sort_assistant_content_blocks(&blocks).unwrap();
    assert_eq!(
        sorted,
        [blocks[1].clone(), blocks[2].clone(), blocks[0].clone()]
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_sorting_an_empty_list_yields_an_empty_list() {
    assert_eq!(sort_assistant_content_blocks(&[]), Ok(Vec::new()));
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_non_object_block_is_rejected_when_sorting() {
    let blocks = vec![json(r#"{"text":"keep"}"#), json("5")];
    assert_eq!(
        sort_assistant_content_blocks(&blocks),
        Err(ContentError::BlockNotObject { index: 1 })
    );
}
