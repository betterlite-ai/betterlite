use std::time::Duration;

use super::super::fixture::{ClientCall, Reply, fixture_probe, now, run};
use super::super::support::{batch, titan_body, vector};
use crate::bedrock_embedding_io::response::EmbeddingReply;
use crate::bedrock_embedding_io::transport::Timeout;
use betterlite_core::api::vector_store_http_write::service::ClientKind;

pub(super) fn ordered_multi_input_result() {
    sync_orders_replies();
    async_keeps_its_timeout();
    each_signature_reads_the_clock();
    empty_batch_still_transforms();
    supplied_client_needs_the_matching_kind();
    a_failure_stops_later_requests();
}

fn sync_orders_replies() {
    let mut outcome_probe = fixture_probe();
    outcome_probe.input = vec![String::from("one"), String::from("two")];
    outcome_probe.timeout = Some(Timeout::Seconds(7.0));
    outcome_probe.replies = vec![
        Reply::ok(&titan_body("[1.5]", 3)),
        Reply::ok(&titan_body("[2.5]", 5)),
    ];
    let outcome = run(&outcome_probe);
    assert_eq!(outcome.factory_requests.len(), 2);
    assert_eq!(
        outcome.factory_requests[0].body,
        b"{\"inputText\":\"one\"}".as_slice()
    );
    assert_eq!(
        outcome.factory_requests[1].body,
        b"{\"inputText\":\"two\"}".as_slice()
    );
    assert_ne!(
        outcome.factory_requests[0].headers["Authorization"],
        outcome.factory_requests[1].headers["Authorization"]
    );
    assert_ordered_reply(batch(outcome.response()));
    assert_eq!(
        outcome.calls,
        vec![
            ClientCall {
                kind: ClientKind::Sync,
                provider: None,
                timeout: Some(Timeout::Seconds(7.0)),
            },
            ClientCall {
                kind: ClientKind::Sync,
                provider: None,
                timeout: Some(Timeout::Seconds(7.0)),
            },
        ]
    );
}

fn assert_ordered_reply(reply: &EmbeddingReply) {
    assert_eq!(reply.object, "list");
    assert_eq!(reply.data.len(), 2);
    assert_eq!(vector(&reply.data[0].embedding), [1.5_f64]);
    assert_eq!(vector(&reply.data[1].embedding), [2.5_f64]);
    assert_eq!(reply.data[1].index, 1);
    assert_eq!(reply.usage.prompt_tokens, 8);
    assert_eq!(reply.usage.completion_tokens, 0);
    assert_eq!(reply.usage.total_tokens, 8);
}

fn async_keeps_its_timeout() {
    let mut outcome_probe = fixture_probe();
    outcome_probe.input = vec![String::from("one"), String::from("two")];
    outcome_probe.timeout = Some(Timeout::Seconds(2.5));
    outcome_probe.aembedding = true;
    outcome_probe.replies = vec![
        Reply::ok(&titan_body("[1.0]", 1)),
        Reply::ok(&titan_body("[2.0]", 1)),
    ];
    let outcome = run(&outcome_probe);
    assert_eq!(outcome.factory_requests.len(), 2);
    assert_eq!(
        outcome.calls,
        vec![
            ClientCall {
                kind: ClientKind::Asynchronous,
                provider: Some(String::from("bedrock")),
                timeout: Some(Timeout::Seconds(2.5)),
            },
            ClientCall {
                kind: ClientKind::Asynchronous,
                provider: Some(String::from("bedrock")),
                timeout: Some(Timeout::Seconds(2.5)),
            },
        ]
    );
    assert_eq!(
        vector(&batch(outcome.response()).data[1].embedding),
        [2.0_f64]
    );
}

fn each_signature_reads_the_clock() {
    let mut outcome_probe = fixture_probe();
    outcome_probe.input = vec![String::from("one"), String::from("two")];
    outcome_probe.clock = vec![
        now(),
        now()
            .checked_add(Duration::from_secs(60))
            .expect("the fixture clock is in range"),
    ];
    outcome_probe.replies = vec![
        Reply::ok(&titan_body("[1.0]", 1)),
        Reply::ok(&titan_body("[2.0]", 1)),
    ];
    let outcome = run(&outcome_probe);
    let dates: Vec<&str> = outcome
        .factory_requests
        .iter()
        .map(|request| request.headers["X-Amz-Date"].as_str())
        .collect();
    assert_eq!(dates, ["20231114T221320Z", "20231114T221420Z"]);
}

fn empty_batch_still_transforms() {
    let mut outcome_probe = fixture_probe();
    outcome_probe.input = Vec::new();
    let outcome = run(&outcome_probe);
    assert!(outcome.factory_requests.is_empty());
    assert!(outcome.calls.is_empty());
    let reply = batch(outcome.response());
    assert_eq!(reply.object, "list");
    assert!(reply.data.is_empty());
    assert_eq!(reply.usage.prompt_tokens, 0);
    assert_eq!(reply.usage.total_tokens, 0);
    assert_eq!(reply.model, "amazon.titan-embed-text-v1");
}

fn supplied_client_needs_the_matching_kind() {
    let mut matching_probe = fixture_probe();
    matching_probe.input = vec![String::from("one"), String::from("two")];
    matching_probe.supplied = Some(ClientKind::Sync);
    matching_probe.replies = vec![
        Reply::ok(&titan_body("[1.0]", 1)),
        Reply::ok(&titan_body("[2.0]", 1)),
    ];
    let matching = run(&matching_probe);
    assert_eq!(matching.supplied_requests.len(), 2);
    assert!(matching.factory_requests.is_empty());
    assert!(matching.calls.is_empty());
    let mut other_kind_probe = fixture_probe();
    other_kind_probe.supplied = Some(ClientKind::Asynchronous);
    other_kind_probe.replies = vec![Reply::ok(&titan_body("[1.0]", 1))];
    let other_kind = run(&other_kind_probe);
    assert!(other_kind.supplied_requests.is_empty());
    assert_eq!(other_kind.calls.len(), 1);
    assert_eq!(other_kind.calls[0].kind, ClientKind::Sync);
    let mut async_matching_probe = fixture_probe();
    async_matching_probe.aembedding = true;
    async_matching_probe.supplied = Some(ClientKind::Asynchronous);
    async_matching_probe.replies = vec![Reply::ok(&titan_body("[1.0]", 1))];
    let async_matching = run(&async_matching_probe);
    assert_eq!(async_matching.supplied_requests.len(), 1);
    assert!(async_matching.calls.is_empty());
}

fn a_failure_stops_later_requests() {
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
    assert_eq!(outcome.error().status(), Some(500));
    assert_eq!(outcome.factory_requests.len(), 2);
}
