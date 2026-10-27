#![cfg(test)]

use crate::extraction::{
    extract,
    tests::{claim, execution, model, page, reader},
};

#[tokio::test]
async fn completed_reads_are_not_pending() {
    let result = extract(
        claim(),
        execution(),
        reader(vec![page(), page()]),
        model(vec![r#"{"reads":[{"span_id":"s"}]}"#, "{}"]),
    )
    .await
    .unwrap();
    assert!(!result.partial);
    assert!(!result.cannot_assess);
}

#[tokio::test]
async fn forced_decision_clears_pending() {
    let result = extract(
        claim(),
        execution(),
        reader(vec![page()]),
        model(vec![
            r#"{"reads":[{"span_id":"s"}],"feedback_page":9}"#,
            "{}",
        ]),
    )
    .await
    .unwrap();
    assert!(!result.partial);
}

#[rstest::rstest]
#[case::partial(true, None)]
#[case::unfinished(false, Some(""))]
#[tokio::test]
async fn incomplete_page_is_partial(#[case] partial: bool, #[case] cursor: Option<&str>) {
    let mut content = page();
    content.partial = partial;
    content.next_cursor = cursor.map(String::from);
    let result = extract(
        claim(),
        execution(),
        reader(vec![content]),
        model(vec!["{}"]),
    )
    .await
    .unwrap();
    assert!(result.partial);
    assert!(!result.cannot_assess);
}
