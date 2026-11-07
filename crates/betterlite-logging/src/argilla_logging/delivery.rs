use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::credentials::ArgillaCredentials;
use super::extraction::{ArgillaItem, ArgillaPayload};
use super::transport::{ArgillaMethod, ArgillaRequest, ArgillaResponse, Transport};
use crate::batch::delivery::{BatchCancelled, BatchDelivery};

// httpx reads a bare timeout number as seconds, so LiteLLM's 60000 is about sixteen hours, not sixty seconds.
pub const ASYNC_TIMEOUT: Duration = Duration::from_mins(1_000);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchMode {
    Sync,
    Async,
}

fn batch_request(
    credentials: &ArgillaCredentials,
    method: ArgillaMethod,
    body: Vec<u8>,
    timeout: Option<Duration>,
) -> ArgillaRequest {
    ArgillaRequest {
        method,
        url: format!(
            "{}/api/v1/datasets/{}/records/bulk",
            credentials.base_url, credentials.dataset_name
        ),
        api_key: credentials.api_key.clone(),
        body: Some(body),
        timeout,
    }
}

fn log_answer(records: usize, response: &ArgillaResponse) {
    if response.status >= 300 {
        tracing::error!(
            status = response.status,
            body = response.body,
            "Argilla HTTP Error"
        );
    } else {
        tracing::debug!(records, "batch of runs successfully created");
    }
}

pub struct ArgillaDelivery<'logger, T> {
    pub transport: &'logger T,
    pub credentials: &'logger ArgillaCredentials,
    pub mode: BatchMode,
}

impl<T: Transport> BatchDelivery<ArgillaItem> for ArgillaDelivery<'_, T> {
    async fn deliver(
        &self,
        batch: &[ArgillaItem],
        settled: &AtomicUsize,
    ) -> Result<Vec<ArgillaItem>, BatchCancelled<ArgillaItem>> {
        let encoded = match self.mode {
            BatchMode::Sync => serde_json::to_vec(batch),
            BatchMode::Async => serde_json::to_vec(&ArgillaPayload {
                items: batch.to_vec(),
            }),
        };
        let Ok(body) = encoded else {
            tracing::error!("Argilla Layer Error - the batch cannot be encoded");
            return Err(BatchCancelled {
                undelivered: batch.to_vec(),
            });
        };
        let (method, timeout) = match self.mode {
            BatchMode::Sync => (ArgillaMethod::Post, None),
            BatchMode::Async => (ArgillaMethod::Put, Some(ASYNC_TIMEOUT)),
        };
        let request = batch_request(self.credentials, method, body, timeout);
        match self.transport.send(request).await {
            Ok(response) if self.mode == BatchMode::Async && response.status >= 300 => {
                log_answer(batch.len(), &response);
                Ok(batch.to_vec())
            }
            Ok(response) => {
                log_answer(batch.len(), &response);
                settled.fetch_add(batch.len(), Ordering::SeqCst);
                Ok(Vec::new())
            }
            Err(error) => {
                tracing::error!(%error, "Argilla Layer Error");
                Err(BatchCancelled {
                    undelivered: batch.to_vec(),
                })
            }
        }
    }
}
