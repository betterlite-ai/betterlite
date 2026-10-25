#![cfg(test)]

use super::*;
use crate::bedrock_records::TextContent;

fn item(text: &str) -> ContentItem {
    let content = TextContent {
        text: Some(text.to_owned()),
        qualifiers: None,
        extra: BTreeMap::new(),
    };
    ContentItem {
        text: Some(content),
        extra: BTreeMap::new(),
    }
}

#[test]
fn bin_pack_preserves_order_within_budget() {
    let batches = bin_pack(&[item("ab"), item("cd"), item("e")], 4);
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].len(), 2);
}

#[test]
fn split_preserves_whitespace_and_qualifiers() {
    let value = item("one two three");
    let (left, right) = split(&[value]).expect("splittable");
    let left_text = left[0].text.as_ref().and_then(|text| text.text.as_deref());
    let right_text = right[0].text.as_ref().and_then(|text| text.text.as_deref());
    assert_eq!(
        format!("{}{}", left_text.unwrap_or(""), right_text.unwrap_or("")),
        "one two three"
    );
}

#[test]
fn merge_selects_intervened_action() {
    let first = ChunkResult {
        response: serde_json::from_str(r#"{"action":"NONE"}"#).unwrap(),
        content: vec![item("a")],
        fragment_group_size: 1,
    };
    let second = ChunkResult {
        response: serde_json::from_str(r#"{"action":"GUARDRAIL_INTERVENED"}"#).unwrap(),
        content: vec![item("b")],
        fragment_group_size: 1,
    };
    assert_eq!(
        merge(&[first, second]).action.as_deref(),
        Some("GUARDRAIL_INTERVENED")
    );
}

#[test]
fn merge_sums_known_counters_and_retains_earlier_extensions() {
    let chunks = [
        ChunkResult {
            response: serde_json::from_str(
                r#"{"usage":{"topicPolicyUnits":2,"future":3},"trace":"first"}"#,
            )
            .unwrap(),
            content: vec![item("safe")],
            fragment_group_size: 1,
        },
        ChunkResult {
            response: serde_json::from_str(
                r#"{"usage":{"topicPolicyUnits":4,"future":5},"outputs":[{"text":"MASK"}]}"#,
            )
            .unwrap(),
            content: vec![item("secret")],
            fragment_group_size: 1,
        },
    ];
    let merged = merge(&chunks);
    assert_eq!(
        merged.usage.unwrap().0.get("topicPolicyUnits"),
        Some(&Some(6))
    );
    assert!(merged.extra.contains_key("trace"));
    assert_eq!(merged.outputs.unwrap()[0].text.as_deref(), Some("safe"));
}

#[test]
fn bin_pack_counts_unicode_characters() {
    assert_eq!(bin_pack(&[item("\u{e9}"), item("\u{732b}")], 2).len(), 1);
}
