#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_http::client_kind::ClientKind;
use secrecy::SecretString;

use super::support::{self, COMPLETED, CREATED, Fixture, Recorder, STREAM_BODY, run_request};
use crate::assistants::operations::{RunCall, RunDispatch, RunInvocation};

// Only a literal true selects the async client or the streaming helper; every other value stays sync and polled.
const PATHS: [(Option<bool>, Option<bool>, ClientKind, bool); 9] = [
    (None, None, ClientKind::Sync, false),
    (Some(false), None, ClientKind::Sync, false),
    (Some(true), None, ClientKind::Asynchronous, false),
    (None, Some(false), ClientKind::Sync, false),
    (Some(false), Some(false), ClientKind::Sync, false),
    (Some(true), Some(false), ClientKind::Asynchronous, false),
    (None, Some(true), ClientKind::Sync, true),
    (Some(false), Some(true), ClientKind::Sync, true),
    (Some(true), Some(true), ClientKind::Asynchronous, true),
];

pub(super) async fn check() {
    picks_the_client_and_helper().await;
    returns_a_supplied_client_as_is().await;
    drops_the_facade_only_arguments().await;
    builds_from_the_litellm_params().await;
    sends_the_provider_bearer_with_the_key_it_has().await;
}

// The SDK adds the api-key whenever the client holds one, and the provider's bearer goes out beside it.
async fn sends_the_provider_bearer_with_the_key_it_has() {
    for (api_key, expected) in [
        (Some(SecretString::from(support::KEY)), Some(support::KEY)),
        (None, None),
    ] {
        let mut fixture = Fixture::new();
        fixture.api_key = api_key;
        fixture.provider = Some(support::provider(Arc::new(support::FixedToken("ad-token"))));
        let recorder = Recorder::start();
        recorder.queue_poll(200, CREATED);
        recorder.queue_poll(200, COMPLETED);
        support::polled(&recorder, &fixture, &run_request()).await;
        let seen = recorder.requests();
        assert_eq!(
            seen[0].headers.get("authorization").map(String::as_str),
            Some("Bearer ad-token")
        );
        assert_eq!(seen[0].headers.get("api-key").map(String::as_str), expected);
    }
}

fn expected_kind(arun_thread: Option<bool>) -> ClientKind {
    if arun_thread == Some(true) {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    }
}

async fn picks_the_client_and_helper() {
    let fixture = Fixture::new();
    for (arun_thread, stream, kind, streaming) in PATHS {
        let dispatch = RunDispatch {
            arun_thread,
            stream,
        };
        let recorder = Recorder::start();
        let run = run_request();
        let outcome = if stream == Some(true) {
            recorder.queue(200, STREAM_BODY);
            let manager = support::streamed(&recorder, &fixture, &run, dispatch, None).await;
            let served = manager.kind;
            manager.consume().await.unwrap();
            (served, true)
        } else {
            recorder.queue_poll(200, CREATED);
            recorder.queue_poll(200, COMPLETED);
            match support::invoked(&recorder, &fixture, &run, dispatch, None).await {
                RunInvocation::Polled(served, _) => (served, false),
                RunInvocation::Streaming(_) => panic!("expected a polled run"),
            }
        };
        assert_eq!(outcome, (kind, streaming));
        let seen = recorder.requests();
        let body = seen[0].body.as_ref().unwrap();
        assert_eq!(body["stream"], JsonValue::Bool(streaming));
    }
}

async fn returns_a_supplied_client_as_is() {
    let fixture = Fixture::new();
    let run = run_request();
    let recorder = Recorder::start();
    recorder.queue_poll(200, CREATED);
    recorder.queue_poll(200, COMPLETED);
    let supplied = support::supplied_client(ClientKind::Sync, &recorder);
    let dispatch = RunDispatch {
        arun_thread: Some(true),
        stream: None,
    };
    let call = RunCall {
        dispatch,
        run: &run,
        handler: None,
    };
    let invocation = support::api(&recorder)
        .run(&fixture.request(), Some(supplied), call)
        .await
        .unwrap();
    let RunInvocation::Polled(kind, reply) = invocation else {
        panic!("expected a polled run");
    };
    assert_eq!(kind, ClientKind::Sync);
    assert_eq!(reply.status, "completed");
    let seen = recorder.requests();
    assert_eq!(
        seen[0].url,
        "https://supplied.test/openai/threads/thread_1/runs?api-version=2024-01-01"
    );
    assert_eq!(seen[0].deadline, Duration::from_secs(42));
    assert_eq!(
        seen[0].headers.get("api-key").map(String::as_str),
        Some("supplied-key")
    );
    assert_eq!(seen[0].headers.get("authorization"), None);
}

async fn drops_the_facade_only_arguments() {
    let mut fixture = Fixture::new();
    fixture.azure_ad_token = Some(SecretString::from("method-token"));
    fixture.timeout = 3.0_f64;
    fixture.max_retries = Some(9);
    let recorder = Recorder::start();
    recorder.queue_poll(200, CREATED);
    recorder.queue_poll(200, COMPLETED);
    support::polled(&recorder, &fixture, &run_request()).await;
    let seen = recorder.requests();
    assert_eq!(
        seen[0].headers.get("api-key").map(String::as_str),
        Some(support::KEY)
    );
    assert_eq!(seen[0].headers.get("authorization"), None);
    assert_eq!(seen[0].deadline, Duration::from_secs(600));
}

async fn builds_from_the_litellm_params() {
    let mut fixture = Fixture::new();
    fixture.params.azure_ad_token = Some(SecretString::from("litellm-token"));
    fixture.params.timeout = Some(12.5_f64);
    fixture.params.max_retries = Some(4);
    for (arun_thread, stream) in [
        (None, None),
        (Some(true), None),
        (None, Some(true)),
        (Some(true), Some(true)),
    ] {
        let recorder = Recorder::start();
        let dispatch = RunDispatch {
            arun_thread,
            stream,
        };
        let run = run_request();
        if stream == Some(true) {
            recorder.queue(200, STREAM_BODY);
            let manager = support::streamed(&recorder, &fixture, &run, dispatch, None).await;
            assert_eq!(manager.kind, expected_kind(arun_thread));
            assert_eq!(manager.read_timeout, Duration::from_millis(12_500));
            assert_eq!(manager.request.deadline, Duration::from_mins(100));
            manager.consume().await.unwrap();
        } else {
            recorder.queue_poll(200, CREATED);
            recorder.queue_poll(200, COMPLETED);
            let invocation = support::invoked(&recorder, &fixture, &run, dispatch, None).await;
            let RunInvocation::Polled(kind, _) = invocation else {
                panic!("expected a polled run");
            };
            assert_eq!(kind, expected_kind(arun_thread));
        }
        sent_with_the_token(&recorder);
    }
}

fn sent_with_the_token(recorder: &Recorder) {
    let seen = recorder.requests();
    assert_eq!(
        seen[0].headers.get("authorization").map(String::as_str),
        Some("Bearer litellm-token")
    );
    assert_eq!(seen[0].headers.get("api-key"), None);
}
