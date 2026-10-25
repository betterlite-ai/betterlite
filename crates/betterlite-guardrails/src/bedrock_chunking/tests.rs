#![cfg(test)]

use super::*;
use crate::bedrock_records::{BedrockGuardrailResponse, BedrockTextContent};

fn item(text: &str) -> BedrockContentItem {
    let content = BedrockTextContent {
        text: Some(text.to_owned()),
        qualifiers: None,
    };
    BedrockContentItem {
        text: Some(content),
    }
}

#[test]
fn bin_pack_preserves_order_within_budget() {
    let batches = bin_pack_bedrock_content(&[item("ab"), item("cd"), item("e")], 4);
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].len(), 2);
}

#[test]
fn split_preserves_whitespace_and_qualifiers() {
    let value = item("one two three");
    let (left, right) = split_bedrock_content(&[value]).expect("splittable");
    let left_text = left[0].text.as_ref().and_then(|text| text.text.as_deref());
    let right_text = right[0].text.as_ref().and_then(|text| text.text.as_deref());
    assert_eq!(
        format!("{}{}", left_text.unwrap_or(""), right_text.unwrap_or("")),
        "one two three"
    );
}

#[test]
fn merge_selects_intervened_action() {
    let first = BedrockContentChunkResult {
        response: BedrockGuardrailResponse {
            action: Some("NONE".to_owned()),
            ..Default::default()
        },
        content: vec![item("a")],
        fragment_group_size: 1,
    };
    let second = BedrockContentChunkResult {
        response: BedrockGuardrailResponse {
            action: Some("GUARDRAIL_INTERVENED".to_owned()),
            ..Default::default()
        },
        content: vec![item("b")],
        fragment_group_size: 1,
    };
    assert_eq!(
        merge_bedrock_guardrail_responses(&[first, second])
            .action
            .as_deref(),
        Some("GUARDRAIL_INTERVENED")
    );
}
