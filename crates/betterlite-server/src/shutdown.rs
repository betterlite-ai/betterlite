use crate::server::ServerError;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(unix)]
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::task::JoinSet;
pub(crate) struct ShutdownSignal {
    #[cfg(unix)]
    terminate: Signal,
    #[cfg(unix)]
    interrupt: Signal,
}
pub(crate) fn build_shutdown_signal() -> Result<ShutdownSignal, ServerError> {
    #[cfg(unix)]
    {
        let terminate =
            signal(SignalKind::terminate()).map_err(|source| ServerError::SignalRegistration {
                signal: "SIGTERM",
                source,
            })?;
        let interrupt =
            signal(SignalKind::interrupt()).map_err(|source| ServerError::SignalRegistration {
                signal: "SIGINT",
                source,
            })?;
        Ok(ShutdownSignal {
            terminate,
            interrupt,
        })
    }
    #[cfg(not(unix))]
    {
        Ok(ShutdownSignal {})
    }
}
impl ShutdownSignal {
    pub(crate) async fn wait(self, shutting_down: Arc<AtomicBool>) {
        #[cfg(unix)]
        {
            let mut signals = JoinSet::new();
            let mut terminate = self.terminate;
            signals.spawn(async move {
                terminate.recv().await;
            });
            let mut interrupt = self.interrupt;
            signals.spawn(async move {
                interrupt.recv().await;
            });
            if let Some(Err(error)) = signals.join_next().await {
                tracing::debug!(error = ?error, "shutdown signal task stopped");
            }
            signals.shutdown().await;
        };
        #[cfg(not(unix))]
        let _ = tokio::signal::ctrl_c().await;
        shutting_down.store(true, Ordering::Release);
        tracing::info!("betterlite server shutting down");
    }
}
