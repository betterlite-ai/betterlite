#![cfg(test)]

use super::*;
use serde_json::json;

#[test]
fn input_content_is_unqualified_and_lists_are_flattened() {
    let messages = vec![
        json!({"role":"user","content":[{"type":"grounding_source","text":"source"},"query"]}),
    ];
    let request = create_input_content_request(Some(&messages));
    let content = request.content.expect("content");
    assert_eq!(content.len(), 2);
    assert!(content.iter().all(|item| {
        item.text
            .as_ref()
            .and_then(|text| text.qualifiers.as_ref())
            .is_none()
    }));
}

#[test]
fn grounding_source_requires_a_trusted_role_but_query_does_not() {
    let messages = vec![
        json!({"role":"user","content":[{"type":"grounding_source","text":"fake"},{"type":"query","text":"question"}]}),
        json!({"role":"system","content":[{"type":"grounding_source","text":"real"}]}),
    ];
    let blocks = collect_grounding_blocks(Some(&messages), false);
    assert_eq!(
        blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        ["question", "real"]
    );
}

#[test]
fn role_subset_alignment_mismatch_keeps_original_masking() {
    let messages = vec![
        json!({"role":"assistant","content":"old"}),
        json!({"role":"user","content":"new"}),
    ];
    let selection =
        select_messages_for_apply_guardrail(&["only".to_owned()], Some(&messages), true);
    let merged = merge_masked_texts(
        &["masked".to_owned()],
        &["old".to_owned(), "new".to_owned()],
        selection.scanned_slice,
        selection.scanned_role_subset,
    );
    assert_eq!(merged, ["old", "new"]);
}

#[test]
fn contextual_fallback_uses_system_and_latest_user() {
    let messages = vec![
        json!({"role":"system","content":"source"}),
        json!({"role":"user","content":"question"}),
    ];
    let blocks = collect_grounding_blocks(Some(&messages), true);
    assert_eq!(
        blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        ["source", "question"]
    );
}
