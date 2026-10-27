#![cfg(test)]
use crate::extraction::{
    extract,
    tests::{claim, execution, model, page, reader},
};

#[tokio::test]
async fn preserves_resolved_part_order_and_root() {
    let mut content = page();
    let root = content.parts[0].clone();
    content.parts.extend(["a", "b"].map(|id| {
        let mut part = root.clone();
        part.span_id = String::from(id);
        part.parent_span_id = String::from("s");
        part
    }));
    let response = r#"{"observations":[{"check_id":"expected_behavior","summary":"supported","evidence":[{"execution_id":"e","span_id":"b","quote":"answer"},{"execution_id":"e","span_id":"a","quote":"answer"},{"execution_id":"e","span_id":"b","quote":"answer"}]}]}"#;
    let result = extract(
        claim(),
        execution(),
        reader(vec![content]),
        model(vec![response]),
    )
    .await
    .unwrap();
    assert_eq!(
        result
            .parts
            .iter()
            .map(|part| part.span_id.as_str())
            .collect::<Vec<_>>(),
        ["b", "a", "s"]
    );
}
