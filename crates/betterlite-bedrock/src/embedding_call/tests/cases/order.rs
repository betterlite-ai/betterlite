use super::super::fixture::{ClientCall, Probe, Reply, run};
use super::super::support::{batch, titan_body, vector};
use crate::bedrock_embedding_io::transport::{ClientKind, Timeout};

pub(super) fn ordered_multi_input_result() {
    sync_orders_replies();
    async_keeps_its_timeout();
    empty_batch_still_transforms();
    supplied_client_needs_the_matching_kind();
    a_failure_stops_later_requests();
}

fn sync_orders_replies() {
    let outcome = run(&Probe {
        input: vec![String::from("one"), String::from("two")],
        timeout: Some(Timeout::Seconds(7.0)),
        replies: vec![
            Reply::ok(&titan_body("[1.5]", 3)),
            Reply::ok(&titan_body("[2.5]", 5)),
        ],
        ..Probe::default()
    });
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
    let reply = batch(outcome.response());
    assert_eq!(reply.object, "list");
    assert_eq!(reply.data.len(), 2);
    assert_eq!(vector(&reply.data[0].embedding), [1.5]);
    assert_eq!(vector(&reply.data[1].embedding), [2.5]);
    assert_eq!(reply.data[1].index, 1);
    assert_eq!(reply.usage.prompt_tokens, 8);
    assert_eq!(reply.usage.completion_tokens, 0);
    assert_eq!(reply.usage.total_tokens, 8);
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

fn async_keeps_its_timeout() {
    let outcome = run(&Probe {
        input: vec![String::from("one"), String::from("two")],
        timeout: Some(Timeout::Seconds(2.5)),
        aembedding: true,
        replies: vec![
            Reply::ok(&titan_body("[1.0]", 1)),
            Reply::ok(&titan_body("[2.0]", 1)),
        ],
        ..Probe::default()
    });
    assert_eq!(outcome.factory_requests.len(), 2);
    assert_eq!(
        outcome.calls,
        vec![
            ClientCall {
                kind: ClientKind::Async,
                provider: Some(String::from("bedrock")),
                timeout: Some(Timeout::Seconds(2.5)),
            },
            ClientCall {
                kind: ClientKind::Async,
                provider: Some(String::from("bedrock")),
                timeout: Some(Timeout::Seconds(2.5)),
            },
        ]
    );
    assert_eq!(vector(&batch(outcome.response()).data[1].embedding), [2.0]);
}

fn empty_batch_still_transforms() {
    let outcome = run(&Probe {
        input: Vec::new(),
        ..Probe::default()
    });
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
    let matching = run(&Probe {
        input: vec![String::from("one"), String::from("two")],
        supplied: Some(ClientKind::Sync),
        replies: vec![
            Reply::ok(&titan_body("[1.0]", 1)),
            Reply::ok(&titan_body("[2.0]", 1)),
        ],
        ..Probe::default()
    });
    assert_eq!(matching.supplied_requests.len(), 2);
    assert!(matching.factory_requests.is_empty());
    assert!(matching.calls.is_empty());
    let other_kind = run(&Probe {
        supplied: Some(ClientKind::Async),
        replies: vec![Reply::ok(&titan_body("[1.0]", 1))],
        ..Probe::default()
    });
    assert!(other_kind.supplied_requests.is_empty());
    assert_eq!(other_kind.calls.len(), 1);
    assert_eq!(other_kind.calls[0].kind, ClientKind::Sync);
    let async_matching = run(&Probe {
        aembedding: true,
        supplied: Some(ClientKind::Async),
        replies: vec![Reply::ok(&titan_body("[1.0]", 1))],
        ..Probe::default()
    });
    assert_eq!(async_matching.supplied_requests.len(), 1);
    assert!(async_matching.calls.is_empty());
}

fn a_failure_stops_later_requests() {
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
    assert_eq!(outcome.error().status(), Some(500));
    assert_eq!(outcome.factory_requests.len(), 2);
    assert!(outcome.outcome.is_err());
}
