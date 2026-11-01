use super::*;

#[test]
fn reasoning_fields_behavior_tests_behavior() {
    let blocks = [encrypted_block("first"), encrypted_block("second")];

    let items = reasoning_items_from_blocks(&blocks);

    assert_eq!(
        items,
        vec![
            ReasoningItem {
                kind: ReasoningItemKind::Reasoning,
                encrypted_content: Some("first".to_owned()),
                summary: Vec::new(),
            },
            ReasoningItem {
                kind: ReasoningItemKind::Reasoning,
                encrypted_content: Some("second".to_owned()),
                summary: Vec::new(),
            },
        ],
        "two adjacent encrypted blocks must not merge into one item"
    );
}
