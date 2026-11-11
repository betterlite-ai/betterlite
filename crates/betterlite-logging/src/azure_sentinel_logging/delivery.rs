use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};

use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::ExposeSecret as _;
use serde::Serialize;

use crate::batch::delivery::{
    BatchCancelled, BatchDelivery, BatchResponse, BatchTransport, DeliverySettings, SendFailure,
    send_batch_with_413_split,
};
use crate::batch::retry::{RequeuePolicy, undelivered_after_http_error};
use crate::datadog_logging::transport::{LOGGING_TIMEOUT, PostRequest};
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

use super::error::SentinelError;
use super::logger::AzureSentinelLogger;

pub const MAX_PAYLOAD_SIZE_BYTES: usize = 1_000_000;
const DELIVERY: DeliverySettings<'static> = DeliverySettings {
    success_status_codes: &[200, 204],
    policy: RequeuePolicy::Undelivered,
    integration_name: "Azure Sentinel",
    drop_error_message: "Azure Sentinel API Error - Payload too large for a single record",
};

pub(crate) struct SentinelBatch<'logger> {
    pub(crate) logger: &'logger AzureSentinelLogger,
    pub(crate) url: &'logger str,
}

struct Sender<'logger> {
    batch: &'logger SentinelBatch<'logger>,
    headers: HeaderMap,
}

impl<E: Serialize + Send + Sync> BatchTransport<E> for Sender<'_> {
    type SendError = SentinelError;
    type LimitError = SentinelError;

    fn send(
        &self,
        batch: &[E],
    ) -> impl Future<Output = Result<BatchResponse, SendFailure<Self::SendError>>> + Send {
        let body = safe_dumps(&batch, DEFAULT_MAX_RECURSE_DEPTH, None);
        let transport = &self.batch.logger.transport;
        let request = (self.batch.url, self.headers.clone());
        async move {
            let encoded = body.map_err(|error| SendFailure::Other(SentinelError::Json(error)))?;
            let response = transport
                .post(PostRequest {
                    url: request.0.to_owned(),
                    headers: request.1,
                    body: encoded.into_bytes(),
                    timeout: LOGGING_TIMEOUT,
                })
                .await
                .map_err(|error| SendFailure::Other(SentinelError::Transport(error)))?;
            Ok(BatchResponse {
                status: response.status,
                detail: str::from_utf8(&response.body)
                    .unwrap_or("<non-UTF-8 body>")
                    .to_owned(),
            })
        }
    }

    fn exceeds_limits(&self, batch: &[E]) -> Result<bool, Self::LimitError> {
        if batch.len() > self.batch.logger.settings.batch.batch_size {
            return Ok(true);
        }
        let encoded = safe_dumps(&batch, DEFAULT_MAX_RECURSE_DEPTH, None)?;
        Ok(encoded.len() > MAX_PAYLOAD_SIZE_BYTES)
    }
}

fn authorization_headers(token: &str) -> Result<HeaderMap, SentinelError> {
    let mut authorization =
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(SentinelError::Authorization)?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(headers)
}

impl SentinelBatch<'_> {
    // a token the destination refused retains what the retry policy keeps; any other failure retains the whole batch
    async fn headers_or_retained<E>(
        &self,
        batch: &[E],
        settled: &AtomicUsize,
    ) -> Result<HeaderMap, Vec<E>>
    where
        E: Clone + Sync,
    {
        let logger = self.logger;
        let prepared = logger
            .tokens
            .bearer(&logger.settings, &*logger.transport, &*logger.clock)
            .await
            .and_then(|token| authorization_headers(token.expose_secret()));
        match prepared {
            Ok(headers) => Ok(headers),
            // httpx raises its status error for every status outside 2xx; a 2xx other than 200 is a plain failure
            Err(SentinelError::TokenStatus { status, detail }) if !(200..300).contains(&status) => {
                let kept = undelivered_after_http_error(
                    batch,
                    status,
                    "Azure Sentinel OAuth token",
                    &detail,
                );
                if kept.is_empty() {
                    settled.fetch_add(batch.len(), Ordering::SeqCst);
                }
                Err(kept)
            }
            Err(error) => {
                tracing::error!(%error, "Azure Sentinel Error getting OAuth token");
                Err(batch.to_vec())
            }
        }
    }
}

impl<E: Serialize + Clone + Send + Sync> BatchDelivery<E> for SentinelBatch<'_> {
    async fn deliver(
        &self,
        batch: &[E],
        settled: &AtomicUsize,
    ) -> Result<Vec<E>, BatchCancelled<E>> {
        let headers = match self.headers_or_retained(batch, settled).await {
            Ok(headers) => headers,
            Err(retained) => return Ok(retained),
        };
        let sender = Sender {
            batch: self,
            headers,
        };
        send_batch_with_413_split(&sender, batch, DELIVERY, settled).await
    }
}
