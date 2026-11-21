use std::sync::Arc;

use betterlite_core::api::realtime_contracts::close::{BackendClose, ClientLoopExit};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::input::{BackendOutcome, ClientOutcome, DirectionWait, LifetimeDocument};
use super::transport::{Frames, Locked as _};

#[derive(Debug, thiserror::Error)]
pub(crate) enum ScriptedFailure {
    #[error("the scripted backend direction failed")]
    Backend,
}

#[derive(Clone, Copy)]
enum Side {
    Client,
    Backend,
}

// Both directions LiteLLM spawns share one script: signal, wait for cancellation, or finish at once
async fn wait_for_turn(
    side: Side,
    lifetime: &LifetimeDocument,
    timeline: &Frames,
    cancellation: &CancellationToken,
    notification: &Notify,
) -> bool {
    let (wait, signals, label) = match side {
        Side::Client => (lifetime.client_wait, lifetime.client_signals, "client"),
        Side::Backend => (lifetime.backend_wait, lifetime.backend_signals, "backend"),
    };
    if wait == DirectionWait::Cancel {
        cancellation.cancelled().await;
        timeline.locked().push(format!("{label}-cancel"));
        return true;
    }
    if wait == DirectionWait::Signal {
        notification.notified().await;
        timeline.locked().push(format!("{label}-signal"));
    }
    if signals {
        timeline.locked().push(format!("{label}-notify"));
        notification.notify_one();
    }
    false
}

// The client direction LiteLLM spawns, scripted: it signals, waits for cancellation, or finishes at once
pub(crate) async fn run_client_direction(
    timeline: Frames,
    cancellation: CancellationToken,
    notification: Arc<Notify>,
    lifetime: &LifetimeDocument,
) -> ClientLoopExit {
    if !wait_for_turn(
        Side::Client,
        lifetime,
        &timeline,
        &cancellation,
        &notification,
    )
    .await
    {
        timeline.locked().push(String::from("client-finish"));
    }
    client_exit(lifetime.client_outcome)
}

// The backend direction LiteLLM spawns, scripted: it waits, signals, then closes or fails
pub(crate) async fn run_backend_direction(
    timeline: Frames,
    cancellation: CancellationToken,
    notification: Arc<Notify>,
    lifetime: &LifetimeDocument,
    close: BackendClose,
) -> Result<BackendClose, ScriptedFailure> {
    let cancelled = wait_for_turn(
        Side::Backend,
        lifetime,
        &timeline,
        &cancellation,
        &notification,
    )
    .await;
    match (cancelled, lifetime.backend_outcome) {
        (true, BackendOutcome::Close) => Ok(close),
        (true, BackendOutcome::Error) => Err(ScriptedFailure::Backend),
        (false, BackendOutcome::Close) => {
            timeline.locked().push(String::from("backend-finish"));
            Ok(close)
        }
        (false, BackendOutcome::Error) => {
            timeline.locked().push(String::from("backend-error"));
            Err(ScriptedFailure::Backend)
        }
    }
}

const fn client_exit(outcome: ClientOutcome) -> ClientLoopExit {
    match outcome {
        ClientOutcome::ClientDisconnected => ClientLoopExit::ClientDisconnected,
        ClientOutcome::BackendClosed => ClientLoopExit::BackendClosed,
    }
}
