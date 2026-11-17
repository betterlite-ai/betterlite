#![cfg(test)]

use super::shared::{Builder, Harness, Kind, chat, describe, ok, request_data};

#[tokio::test]
async fn returns_the_original_chunks_when_the_assembly_is_not_a_model_response() {
    let mut harness = Harness::new();
    harness.assembly.builder = Builder::NotAModel;
    let mut request = request_data("{}");
    let (sent, result) = harness
        .run(
            Kind::Mask,
            &mut request,
            ok(vec![chat("Alice"), chat("Bob")]),
        )
        .await;
    result.unwrap();
    assert_eq!(sent, [describe(&chat("Alice")), describe(&chat("Bob"))]);
}
