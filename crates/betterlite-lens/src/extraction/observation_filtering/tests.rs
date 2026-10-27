#![cfg(test)]
use crate::extraction::{
    extract,
    tests::{OBSERVATION, claim, execution, model, page, reader},
};
use rstest::rstest;

#[rstest]
#[case::disabled("expected_behavior", "disabled")]
#[case::wrong_execution("\"execution_id\":\"e\"", "\"execution_id\":\"other\"")]
#[case::wrong_span("\"span_id\":\"s\"", "\"span_id\":\"absent\"")]
#[case::wrong_quote("answer", "Answer")]
#[tokio::test]
async fn rejects_unsupported_observation(#[case] from: &str, #[case] to: &str) {
    let response = OBSERVATION.replace(from, to);
    let result = extract(
        claim(),
        execution(),
        reader(vec![page()]),
        model(vec![&response]),
    )
    .await
    .unwrap();
    assert!(result.observations.is_empty());
    assert!(result.partial && result.cannot_assess);
}

#[tokio::test]
async fn omission_crossing_is_invalid() {
    let mut content = page();
    content.parts[0].content = String::from("an\n[... content omitted ...]\nswer");
    let result = extract(
        claim(),
        execution(),
        reader(vec![content]),
        model(vec![OBSERVATION]),
    )
    .await
    .unwrap();
    assert!(result.observations.is_empty());
    assert!(result.partial);
}
