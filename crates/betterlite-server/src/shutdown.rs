use crate::server::ServerError;
use futures_util::future::{Either, select};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(unix)]
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::sync::watch;
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
    pub(crate) async fn wait(
        mut self,
        shutting_down: Arc<AtomicBool>,
        sender: watch::Sender<bool>,
    ) {
        #[cfg(unix)]
        {
            let terminate = self.terminate.recv();
            let interrupt = self.interrupt.recv();
            tokio::pin!(terminate);
            tokio::pin!(interrupt);
            match select(terminate, interrupt).await {
                Either::Left((_result, _)) | Either::Right((_result, _)) => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        shutting_down.store(true, Ordering::Release);
        let _send_result = sender.send(true);
    }
}
