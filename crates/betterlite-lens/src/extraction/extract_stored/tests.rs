#![cfg(test)]
use crate::extraction::tests::part;
use crate::extraction::{
    extract,
    tests::{OBSERVATION, claim, execution, model, page, reader},
};

#[tokio::test]
async fn catalog_iteration_retains_each_review() {
    let mut large = page();
    large.parts.extend((0_u32..200).map(|index| {
        let mut child = part();
        child.span_id = format!("z{index:03}");
        child.parent_span_id = String::from("s");
        child.content = "x".repeat(160);
        child
    }));
    let result = extract(
        claim(),
        execution(),
        reader(vec![large]),
        model(vec![OBSERVATION, OBSERVATION]),
    )
    .await
    .unwrap();
    assert_eq!(result.observations.len(), 2);
    assert_eq!(result.parts.len(), 1);
    assert!(!result.partial);
}
