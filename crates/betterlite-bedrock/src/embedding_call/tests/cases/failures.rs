use super::super::fixture::{Reply, fixture_probe, run};
use super::super::support::{phases, titan_body};
use crate::bedrock_embedding_io::error::EmbeddingIoError;
use crate::embedding_call::error::EmbeddingCallError;

pub(super) fn embedding_transport_error() {
    an_http_status_stops_the_loop();
    a_timeout_is_the_handler_error();
    a_transport_failure_keeps_its_class();
    a_decode_failure_stops_the_loop();
    an_unmapped_amazon_model_fails_the_same_way();
}

fn an_http_status_stops_the_loop() {
    let mut outcome_probe = fixture_probe();
    outcome_probe.input = vec![
        String::from("one"),
        String::from("two"),
        String::from("three"),
    ];
    outcome_probe.replies = vec![
        Reply::ok(&titan_body("[1.0]", 1)),
        Reply::status(500, &[], "boom"),
    ];
    let outcome = run(&outcome_probe);
    assert!(matches!(
        outcome.error(),
        EmbeddingCallError::Embedding(EmbeddingIoError::Provider { status: 500, .. })
    ));
    assert_eq!(outcome.error().status(), Some(500));
    assert_eq!(outcome.factory_requests.len(), 2);
    // the first item logs its reply, the failed one logs only its pre call
    assert_eq!(
        phases(&outcome.snapshots),
        ["pre_api_call", "post_api_call", "pre_api_call"]
    );
}

fn a_timeout_is_the_handler_error() {
    let mut synchronous_probe = fixture_probe();
    synchronous_probe.replies = vec![Reply::Timeout];
    let synchronous = run(&synchronous_probe);
    assert!(matches!(
        synchronous.error(),
        EmbeddingCallError::Embedding(EmbeddingIoError::Handler(_))
    ));
    assert_eq!(synchronous.error().status(), Some(408));
    let mut asynchronous_probe = fixture_probe();
    asynchronous_probe.aembedding = true;
    asynchronous_probe.replies = vec![Reply::Timeout];
    let asynchronous = run(&asynchronous_probe);
    assert_eq!(asynchronous.error().status(), Some(408));
    assert_eq!(asynchronous.calls.len(), 1);
}

fn a_transport_failure_keeps_its_class() {
    let mut outcome_probe = fixture_probe();
    outcome_probe.replies = vec![Reply::Failed];
    let outcome = run(&outcome_probe);
    assert!(matches!(
        outcome.error(),
        EmbeddingCallError::Embedding(EmbeddingIoError::Transport(_))
    ));
    assert_eq!(outcome.error().status(), None);
}

fn a_decode_failure_stops_the_loop() {
    let mut outcome_probe = fixture_probe();
    outcome_probe.input = vec![String::from("one"), String::from("two")];
    outcome_probe.replies = vec![Reply::ok("[]")];
    let outcome = run(&outcome_probe);
    assert!(matches!(
        outcome.error(),
        EmbeddingCallError::Embedding(EmbeddingIoError::Decode { .. })
    ));
    assert_eq!(outcome.error().status(), None);
    assert_eq!(outcome.factory_requests.len(), 1);
}

fn an_unmapped_amazon_model_fails_the_same_way() {
    for aembedding in [false, true] {
        let mut outcome_probe = fixture_probe();
        outcome_probe.model = String::from("amazon.titan-embed-text-v3");
        outcome_probe.aembedding = aembedding;
        let outcome = run(&outcome_probe);
        assert!(matches!(outcome.error(), EmbeddingCallError::Unmapped));
        assert_eq!(
            outcome.error().to_string(),
            "Unable to map Bedrock request to provider"
        );
        assert!(outcome.factory_requests.is_empty());
    }
}
