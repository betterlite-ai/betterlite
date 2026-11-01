use super::*;

#[test]
fn reasoning_fields_behavior_tests_behavior() {
    let blocks = [thinking_block("first"), thinking_block("second")];

    let items = reasoning_items_from_blocks(&blocks);

    assert_eq!(
        items,
        vec![ReasoningItem {
            kind: ReasoningItemKind::Reasoning,
            encrypted_content: None,
            summary: vec![
                ReasoningSummaryTextBlock {
                    kind: SummaryBlockKind::SummaryText,
                    text: "first".to_owned(),
                },
                ReasoningSummaryTextBlock {
                    kind: SummaryBlockKind::SummaryText,
                    text: "second".to_owned(),
                },
            ],
        }]
    );
}
