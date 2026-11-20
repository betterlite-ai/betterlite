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

// Both directions LiteLLM spawns share one script: signal, wait for cancellation, or finish at once
async fn wait_for_turn(
    label: &str,
    wait: DirectionWait,
    signals: bool,
    timeline: &Frames,
    cancellation: &CancellationToken,
    notification: &Notify,
) -> bool {
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
    if wait_for_turn(
        "client",
        lifetime.client_wait,
        lifetime.client_signals,
        &timeline,
        &cancellation,
        &notification,
    )
    .await
    {
        return client_exit(lifetime.client_outcome);
    }
    timeline.locked().push(String::from("client-finish"));
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
    if wait_for_turn(
        "backend",
        lifetime.backend_wait,
        lifetime.backend_signals,
        &timeline,
        &cancellation,
        &notification,
    )
    .await
    {
        return match lifetime.backend_outcome {
            BackendOutcome::Close => Ok(close),
            BackendOutcome::Error => Err(ScriptedFailure::Backend),
        };
    }
    match lifetime.backend_outcome {
        BackendOutcome::Close => {
            timeline.locked().push(String::from("backend-finish"));
            Ok(close)
        }
        BackendOutcome::Error => {
            timeline.locked().push(String::from("backend-error"));
            Err(ScriptedFailure::Backend)
        }
    }
}

fn client_exit(outcome: ClientOutcome) -> ClientLoopExit {
    match outcome {
        ClientOutcome::ClientDisconnected => ClientLoopExit::ClientDisconnected,
        ClientOutcome::BackendClosed => ClientLoopExit::BackendClosed,
    }
}
