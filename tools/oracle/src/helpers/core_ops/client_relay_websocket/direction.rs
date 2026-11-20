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

// The client direction LiteLLM spawns, scripted: it signals, waits for cancellation, or finishes at once
pub(crate) async fn run_client_direction(
    timeline: Frames,
    cancellation: CancellationToken,
    notification: Arc<Notify>,
    lifetime: &LifetimeDocument,
) -> ClientLoopExit {
    if lifetime.client_wait == DirectionWait::Cancel {
        cancellation.cancelled().await;
        timeline.locked().push(String::from("client-cancel"));
        return client_exit(lifetime.client_outcome);
    }
    if lifetime.client_wait == DirectionWait::Signal {
        notification.notified().await;
        timeline.locked().push(String::from("client-signal"));
    }
    if lifetime.client_signals {
        timeline.locked().push(String::from("client-notify"));
        notification.notify_one();
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
    if lifetime.backend_wait == DirectionWait::Cancel {
        cancellation.cancelled().await;
        timeline.locked().push(String::from("backend-cancel"));
        return match lifetime.backend_outcome {
            BackendOutcome::Close => Ok(close),
            BackendOutcome::Error => Err(ScriptedFailure::Backend),
        };
    }
    if lifetime.backend_wait == DirectionWait::Signal {
        notification.notified().await;
        timeline.locked().push(String::from("backend-signal"));
    }
    if lifetime.backend_signals {
        timeline.locked().push(String::from("backend-notify"));
        notification.notify_one();
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
