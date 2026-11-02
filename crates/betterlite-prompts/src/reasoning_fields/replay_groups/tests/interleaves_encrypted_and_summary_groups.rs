use super::*;

#[test]
fn reasoning_fields_behavior_tests_behavior() {
    let blocks = [
        thinking_block("before"),
        encrypted_block("mid"),
        thinking_block("after-one"),
        thinking_block("after-two"),
    ];

    let items = reasoning_items_from_blocks(&blocks);

    assert_eq!(
        items,
        vec![
            ReasoningItem {
                kind: ReasoningItemKind::Reasoning,
                encrypted_content: None,
                summary: vec![ReasoningSummaryTextBlock {
                    kind: SummaryBlockKind::SummaryText,
                    text: "before".to_owned(),
                }],
            },
            ReasoningItem {
                kind: ReasoningItemKind::Reasoning,
                encrypted_content: Some("mid".to_owned()),
                summary: Vec::new(),
            },
            ReasoningItem {
                kind: ReasoningItemKind::Reasoning,
                encrypted_content: None,
                summary: vec![
                    ReasoningSummaryTextBlock {
                        kind: SummaryBlockKind::SummaryText,
                        text: "after-one".to_owned(),
                    },
                    ReasoningSummaryTextBlock {
                        kind: SummaryBlockKind::SummaryText,
                        text: "after-two".to_owned(),
                    },
                ],
            },
        ],
        "the two summary runs on either side of the encrypted block stay separate items"
    );
}
