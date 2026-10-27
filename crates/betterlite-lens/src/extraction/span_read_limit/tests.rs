#![cfg(test)]
use crate::analysis::ReadContent;
use crate::extraction::{
    extract,
    tests::{OBSERVATION, claim, execution, model, page, reader},
};
use rstest::rstest;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[rstest]
#[case::missing(r#"{"reads":[{"span_id":"missing"}]}"#, 1)]
#[case::repeated(r#"{"reads":[{"span_id":"s"}]}"#, 2)]
#[tokio::test]
async fn unavailable_read_forces_final(#[case] request: &str, #[case] pages: usize) {
    let replies = if pages == 1 {
        vec![request, OBSERVATION]
    } else {
        vec![request, request, OBSERVATION]
    };
    let result = extract(
        claim(),
        execution(),
        reader(vec![page(); pages]),
        model(replies),
    )
    .await
    .unwrap();
    assert_eq!(result.observations.len(), 1);
}

#[tokio::test]
async fn rejects_more_than_two_reads_and_repairs() {
    let result = extract(
        claim(),
        execution(),
        reader(vec![page()]),
        model(vec![
            r#"{"reads":[{"span_id":"s"},{"span_id":"s"},{"span_id":"s"}]}"#,
            OBSERVATION,
        ]),
    )
    .await
    .unwrap();
    assert_eq!(result.observations.len(), 1);
    assert!(!result.partial);
}

#[tokio::test]
async fn fetch_uses_previous_cursor_and_filters_span() {
    let calls = AtomicUsize::new(0);
    let read: ReadContent = Arc::new(move |id, cursor, offset| {
        assert_eq!(id, "e");
        let mut content = page();
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let mut child = content.parts[0].clone();
            child.span_id = String::from("z");
            content.parts.push(child);
        } else {
            assert_eq!((cursor.as_str(), offset), ("s", 1));
            content.parts[0].span_id = String::from("z");
            content.parts[0].content = String::from("fresh");
        }
        Box::pin(async move { Ok(content) })
    });
    let result = extract(
        claim(),
        execution(),
        read,
        model(vec![
            r#"{"reads":[{"span_id":"z","offset":1}]}"#,
            OBSERVATION,
        ]),
    )
    .await
    .unwrap();
    assert_eq!(result.observations.len(), 1);
}

#[tokio::test]
async fn two_reads_each_turn() {
    let calls = Arc::new(AtomicUsize::new(0));
    let recorded = Arc::clone(&calls);
    let read: ReadContent = Arc::new(move |_, _, offset| {
        recorded.fetch_add(1, Ordering::SeqCst);
        let mut content = page();
        content.parts[0].content = format!("answer {offset}");
        Box::pin(async move { Ok(content) })
    });
    let result = extract(
        claim(),
        execution(),
        read,
        model(vec![
            r#"{"reads":[{"span_id":"s","offset":1},{"span_id":"s","offset":2}]}"#,
            r#"{"reads":[{"span_id":"s","offset":3},{"span_id":"s","offset":4}]}"#,
            OBSERVATION,
        ]),
    )
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 5);
    assert_eq!(result.observations.len(), 1);
    assert!(!result.partial);
}
