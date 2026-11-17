use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use secrecy::ExposeSecret as _;
use subtle::ConstantTimeEq as _;
use tokio::sync::Mutex as AsyncMutex;

use super::Inner;
use crate::batch::delivery::{BatchCancelled, BatchDelivery};
use crate::http_analytics_loggers::delivery::DELIVERY_DEADLINE;
use crate::zerobus::client::types::{ZerobusConnection, ZerobusIngestFailure};
use crate::zerobus::client::{Clock, ZerobusIngestClient};
use crate::zerobus::row::TraceRow;

// B4003's client is concrete, so the logger reads it through this seam to keep its queue testable
#[async_trait]
pub trait RowIngest: Send + Sync {
    fn connection(&self) -> &ZerobusConnection;

    async fn ingest(&self, rows: &[TraceRow]) -> Result<(), ZerobusIngestFailure>;
}

#[async_trait]
impl RowIngest for ZerobusIngestClient {
    fn connection(&self) -> &ZerobusConnection {
        &self.connection
    }

    async fn ingest(&self, rows: &[TraceRow]) -> Result<(), ZerobusIngestFailure> {
        Self::insert(self, rows).await
    }
}

pub struct ClientFactory {
    http: ProxiedClient,
    clock: Clock,
    timeout: Duration,
}

impl ClientFactory {
    #[must_use]
    pub fn new(http: ProxiedClient, clock: Clock) -> Self {
        Self {
            http,
            clock,
            timeout: DELIVERY_DEADLINE,
        }
    }

    pub(super) fn build(&self, connection: ZerobusConnection) -> Arc<dyn RowIngest> {
        Arc::new(ZerobusIngestClient {
            connection,
            http: self.http.clone(),
            clock: Arc::clone(&self.clock),
            timeout: self.timeout,
            token: AsyncMutex::new(None),
        })
    }
}

pub(super) fn same_connection(left: &ZerobusConnection, right: &ZerobusConnection) -> bool {
    left.workspace_url == right.workspace_url
        && left.workspace_id == right.workspace_id
        && left.server_endpoint == right.server_endpoint
        && left.client_id == right.client_id
        && left.table_name == right.table_name
        && bool::from(
            left.client_secret
                .expose_secret()
                .as_bytes()
                .ct_eq(right.client_secret.expose_secret().as_bytes()),
        )
}

pub(super) struct ClientDelivery<'inner> {
    pub(super) inner: &'inner Inner,
}

impl BatchDelivery<TraceRow> for ClientDelivery<'_> {
    async fn deliver(
        &self,
        batch: &[TraceRow],
        settled: &AtomicUsize,
    ) -> Result<Vec<TraceRow>, BatchCancelled<TraceRow>> {
        if batch.is_empty() {
            return Ok(Vec::new());
        }
        let client = match self.inner.client() {
            Ok(client) => client,
            Err(error) => {
                tracing::error!(
                    %error,
                    rows = batch.len(),
                    "zerobus: the connection cannot be resolved, keeping the rows for the next flush"
                );
                return Ok(batch.to_vec());
            }
        };
        match client.ingest(batch).await {
            Ok(()) => {
                settled.fetch_add(batch.len(), Ordering::SeqCst);
                Ok(Vec::new())
            }
            Err(failure) if failure.retryable => {
                tracing::warn!(
                    rows = batch.len(),
                    detail = %failure.detail,
                    "zerobus: the insert can be retried, keeping the rows for the next flush"
                );
                Ok(batch.to_vec())
            }
            Err(failure) => {
                tracing::error!(
                    rows = batch.len(),
                    detail = %failure.detail,
                    "zerobus: dropping rows, the insert cannot be retried"
                );
                settled.fetch_add(batch.len(), Ordering::SeqCst);
                Ok(Vec::new())
            }
        }
    }
}
