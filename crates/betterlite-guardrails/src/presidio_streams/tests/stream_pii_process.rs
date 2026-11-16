#![cfg(test)]

use super::shared::{Harness, Kind, chat, ok, tokens_request};

#[tokio::test]
async fn the_assembled_response_is_unmasked_not_masked() {
    let harness = Harness::new();
    let mut request = tokens_request();
    let events = ok(vec![chat("Hi <PERSON_1>")]);
    let (sent, result) = harness.run(Kind::Unmask, &mut request, events).await;
    result.unwrap();
    assert!(sent[0].contains(r#""content":"Hi Alice""#));
    assert!(!sent[0].contains("MASKED"));
    let metadata = request.get("metadata").unwrap().get();
    assert!(!metadata.contains("standard_logging_guardrail_information"));
}
