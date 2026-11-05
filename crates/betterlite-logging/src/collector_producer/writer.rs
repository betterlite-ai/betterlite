use super::{
    Outage,
    transport::{Connector, peer_closed},
};
use std::sync::{Arc, PoisonError};
use std::time::Duration;
use tokio::io::AsyncWriteExt as _;
use tokio::sync::mpsc;
use tokio::time::{Instant, timeout};
use tokio_util::sync::CancellationToken;

pub(super) struct Writer {
    pub(super) outage: Arc<Outage>,
    pub(super) shutdown: CancellationToken,
    pub(super) connect_timeout: Duration,
}

impl Writer {
    pub(super) async fn run<Connect>(self, mut receiver: mpsc::Receiver<Vec<u8>>, connect: Connect)
    where
        Connect: Connector,
    {
        let mut connection = None;
        while let Some(line) = receiver.recv().await {
            let sent = self
                .shutdown
                .run_until_cancelled(self.send(&line, &mut connection, &connect))
                .await
                .unwrap_or(false);
            if !sent {
                // Dropping the socket first prevents a partial line from being counted twice.
                connection = None;
                self.outage
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .stats
                    .connected = false;
                self.outage.apply(line, "collector unavailable").await;
            }
        }
        drop(connection);
        self.outage
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stats
            .connected = false;
    }

    async fn send<Connect>(
        &self,
        line: &[u8],
        connection: &mut Option<Connect::Stream>,
        connect: &Connect,
    ) -> bool
    where
        Connect: Connector,
    {
        if let Some(stream) = connection
            && peer_closed(stream).await
        {
            *connection = None;
        }
        if connection.is_none() {
            *connection = self.connect(connect).await;
        }
        let Some(stream) = connection else {
            return false;
        };
        let mut offset = 0;
        while let Some(remaining) = line.get(offset..).filter(|remaining| !remaining.is_empty()) {
            match stream.write(remaining).await {
                Ok(0) => {
                    self.backoff();
                    return false;
                }
                Ok(written) => offset = offset.saturating_add(written),
                Err(error) => {
                    tracing::warn!(%error, "collector write failed");
                    self.backoff();
                    return false;
                }
            }
        }
        let mut state = self
            .outage
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.stats.sent = state.stats.sent.saturating_add(1);
        true
    }

    async fn connect<Connect>(&self, connect: &Connect) -> Option<Connect::Stream>
    where
        Connect: Connector,
    {
        {
            let mut state = self
                .outage
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            state.stats.connected = false;
            if state.reconnect_at.is_some_and(|at| Instant::now() < at) {
                return None;
            }
        }
        if let Ok(Ok(stream)) = timeout(self.connect_timeout, connect.open()).await {
            self.outage
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .stats
                .connected = true;
            tracing::info!("collector connected");
            Some(stream)
        } else {
            tracing::warn!("collector connection unavailable; entering backoff");
            self.backoff();
            None
        }
    }

    fn backoff(&self) {
        let mut state = self
            .outage
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.reconnect_at = Instant::now().checked_add(Duration::from_secs(1));
    }
}
