use super::super::fixture::{Probe, Reply, run};
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
    let outcome = run(&Probe {
        input: vec![
            String::from("one"),
            String::from("two"),
            String::from("three"),
        ],
        replies: vec![
            Reply::ok(&titan_body("[1.0]", 1)),
            Reply::status(500, &[], "boom"),
        ],
        ..Probe::default()
    });
    assert!(matches!(
        outcome.error(),
        EmbeddingCallError::Embedding(EmbeddingIoError::Provider { status: 500, .. })
    ));
    assert_eq!(outcome.error().status(), Some(500));
    assert_eq!(outcome.factory_requests.len(), 2);
    assert_eq!(phases(&outcome.snapshots), ["pre_api_call"]);
    assert!(outcome.original_response().is_none());
}

fn a_timeout_is_the_handler_error() {
    let synchronous = run(&Probe {
        replies: vec![Reply::Timeout],
        ..Probe::default()
    });
    assert!(matches!(
        synchronous.error(),
        EmbeddingCallError::Embedding(EmbeddingIoError::Handler(_))
    ));
    assert_eq!(synchronous.error().status(), Some(408));
    let asynchronous = run(&Probe {
        aembedding: true,
        replies: vec![Reply::Timeout],
        ..Probe::default()
    });
    assert_eq!(asynchronous.error().status(), Some(408));
    assert_eq!(asynchronous.calls.len(), 1);
}

fn a_transport_failure_keeps_its_class() {
    let outcome = run(&Probe {
        replies: vec![Reply::Failed],
        ..Probe::default()
    });
    assert!(matches!(
        outcome.error(),
        EmbeddingCallError::Embedding(EmbeddingIoError::Transport(_))
    ));
    assert_eq!(outcome.error().status(), None);
}

fn a_decode_failure_stops_the_loop() {
    let outcome = run(&Probe {
        input: vec![String::from("one"), String::from("two")],
        replies: vec![Reply::ok("[]")],
        ..Probe::default()
    });
    assert!(matches!(
        outcome.error(),
        EmbeddingCallError::Embedding(EmbeddingIoError::Decode { .. })
    ));
    assert_eq!(outcome.error().status(), None);
    assert_eq!(outcome.factory_requests.len(), 1);
}

fn an_unmapped_amazon_model_fails_the_same_way() {
    for aembedding in [false, true] {
        let outcome = run(&Probe {
            model: String::from("amazon.titan-embed-text-v3"),
            aembedding,
            ..Probe::default()
        });
        assert!(matches!(outcome.error(), EmbeddingCallError::Unmapped));
        assert_eq!(
            outcome.error().to_string(),
            "Unable to map Bedrock request to provider"
        );
        assert!(outcome.factory_requests.is_empty());
    }
}
