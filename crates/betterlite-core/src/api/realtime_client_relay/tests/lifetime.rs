use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::{Barrier, Notify};
use tokio_util::sync::CancellationToken;

use crate::api::realtime_backend_relay::relay_callbacks;
use crate::api::realtime_client_relay::RealtimeClientRelay;
use crate::api::realtime_contracts::close::{BackendClose, ClientLoopExit};

use super::harness::*;

#[derive(Debug, thiserror::Error)]
enum ScriptedFailure {
    #[error("scripted backend failure")]
    Backend,
}

struct Lifetime {
    result: Result<(), ScriptedFailure>,
    outcome: Outcome,
}

fn policy_close() -> BackendClose {
    BackendClose {
        code: 1008,
        reason: String::from("policy"),
    }
}

async fn lifespan<ClientDirection, BackendDirection>(
    mut fixture: Fixture,
    client_direction: ClientDirection,
    backend_direction: BackendDirection,
    cancellation: &CancellationToken,
) -> Lifetime
where
    ClientDirection: Future<Output = ClientLoopExit> + Send,
    BackendDirection: Future<Output = Result<BackendClose, ScriptedFailure>> + Send,
{
    let refs = guardrail_refs(&fixture.guardrails);
    let callbacks = relay_callbacks(&refs);
    let result = {
        let mut relay = RealtimeClientRelay {
            state: &mut fixture.state,
            provider: fixture.provider.as_ref(),
            guardrails: &refs,
            callbacks,
            redactor: fixture.redactor.as_ref(),
        };
        relay
            .bidirectional_forward(client_direction, backend_direction, cancellation)
            .await
    };
    Lifetime {
        result,
        outcome: Outcome {
            exit: ClientLoopExit::ClientDisconnected,
            state: fixture.state,
            client: fixture.client,
            backend: fixture.backend,
            logging: fixture.logging,
            normalizer: fixture.normalizer,
        },
    }
}

#[tokio::test]
async fn realtime_bidirectional_lifetime() {
    let (logs, _guard) = capture();
    client_disconnect_returns_without_closing().await;
    backend_completion_closes_the_client().await;
    a_client_backend_closed_waits_for_the_backend().await;
    simultaneous_completion_still_closes().await;
    a_failing_backend_cancels_and_joins_the_client().await;
    a_failing_backend_after_backend_closed_skips_the_close().await;
    assert!(logs.is_empty());
}

async fn client_disconnect_returns_without_closing() {
    let cancellation = CancellationToken::new();
    let backend_token = cancellation.clone();
    let client_joined = Arc::new(AtomicBool::new(false));
    let backend_joined = Arc::new(AtomicBool::new(false));
    let client_flag = Arc::clone(&client_joined);
    let backend_flag = Arc::clone(&backend_joined);
    let lifetime = lifespan(
        Fixture::default(),
        async move {
            client_flag.store(true, Ordering::SeqCst);
            ClientLoopExit::ClientDisconnected
        },
        async move {
            backend_token.cancelled().await;
            backend_flag.store(true, Ordering::SeqCst);
            Ok(policy_close())
        },
        &cancellation,
    )
    .await;
    lifetime.result.unwrap();
    assert!(lifetime.outcome.closes().is_empty());
    assert!(client_joined.load(Ordering::SeqCst));
    assert!(backend_joined.load(Ordering::SeqCst));
}

async fn backend_completion_closes_the_client() {
    let cancellation = CancellationToken::new();
    let client_token = cancellation.clone();
    let client_joined = Arc::new(AtomicBool::new(false));
    let client_flag = Arc::clone(&client_joined);
    let lifetime = lifespan(
        Fixture::default(),
        async move {
            client_token.cancelled().await;
            client_flag.store(true, Ordering::SeqCst);
            ClientLoopExit::BackendClosed
        },
        async move { Ok(policy_close()) },
        &cancellation,
    )
    .await;
    lifetime.result.unwrap();
    assert_eq!(
        lifetime.outcome.closes(),
        vec![(1008, Some(String::from("policy")))]
    );
    assert!(client_joined.load(Ordering::SeqCst));
}

async fn a_client_backend_closed_waits_for_the_backend() {
    let cancellation = CancellationToken::new();
    let backend_release = Arc::new(Notify::new());
    let client_release = Arc::clone(&backend_release);
    let backend_gate = Arc::clone(&backend_release);
    let lifetime = lifespan(
        Fixture::default(),
        async move {
            client_release.notify_one();
            ClientLoopExit::BackendClosed
        },
        async move {
            backend_gate.notified().await;
            Ok(policy_close())
        },
        &cancellation,
    )
    .await;
    lifetime.result.unwrap();
    assert_eq!(
        lifetime.outcome.closes(),
        vec![(1008, Some(String::from("policy")))]
    );
}

async fn simultaneous_completion_still_closes() {
    let cancellation = CancellationToken::new();
    let barrier = Arc::new(Barrier::new(2));
    let client_barrier = Arc::clone(&barrier);
    let backend_barrier = Arc::clone(&barrier);
    let lifetime = lifespan(
        Fixture::default(),
        async move {
            client_barrier.wait().await;
            ClientLoopExit::BackendClosed
        },
        async move {
            backend_barrier.wait().await;
            Ok(policy_close())
        },
        &cancellation,
    )
    .await;
    lifetime.result.unwrap();
    assert_eq!(
        lifetime.outcome.closes(),
        vec![(1008, Some(String::from("policy")))]
    );
}

async fn a_failing_backend_cancels_and_joins_the_client() {
    let cancellation = CancellationToken::new();
    let client_token = cancellation.clone();
    let client_joined = Arc::new(AtomicBool::new(false));
    let client_flag = Arc::clone(&client_joined);
    let lifetime = lifespan(
        Fixture::default(),
        async move {
            client_token.cancelled().await;
            client_flag.store(true, Ordering::SeqCst);
            ClientLoopExit::BackendClosed
        },
        async move { Err(ScriptedFailure::Backend) },
        &cancellation,
    )
    .await;
    assert!(matches!(lifetime.result, Err(ScriptedFailure::Backend)));
    assert!(lifetime.outcome.closes().is_empty());
    assert!(client_joined.load(Ordering::SeqCst));
}

async fn a_failing_backend_after_backend_closed_skips_the_close() {
    let cancellation = CancellationToken::new();
    let lifetime = lifespan(
        Fixture::default(),
        async move { ClientLoopExit::BackendClosed },
        async move { Err(ScriptedFailure::Backend) },
        &cancellation,
    )
    .await;
    assert!(matches!(lifetime.result, Err(ScriptedFailure::Backend)));
    assert!(lifetime.outcome.closes().is_empty());
}
