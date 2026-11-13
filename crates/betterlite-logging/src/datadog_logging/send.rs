use std::future::Future;
use std::io::Write as _;
use std::sync::atomic::AtomicUsize;

use flate2::Compression;
use flate2::write::GzEncoder;
use http::header::{CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;
use thiserror::Error;

use crate::batch::delivery::{
    BatchCancelled, BatchDelivery, BatchResponse, BatchTransport, DeliverySettings, SendFailure,
    send_batch_with_413_split,
};
use crate::batch::retry::RequeuePolicy;
use crate::datadog::payloads::constants::DATADOG_413_ERROR;
use crate::datadog::payloads::types::DatadogPayload;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

use super::error::DatadogError;
use super::logger::DatadogLogger;
use super::settings::{DD_MAX_BATCH_SIZE, DD_MAX_PAYLOAD_SIZE_BYTES};
use super::transport::{LOGGING_TIMEOUT, PostRequest, TransportFailure, TransportResponse};

const API_KEY_HEADER: HeaderName = HeaderName::from_static("dd-api-key");
const SUCCESS_STATUS_CODES: [u16; 1] = [202];

#[derive(Debug, Error)]
pub enum SendError {
    #[error("the Datadog API answered with status {status}")]
    Status { status: u16, detail: String },
    #[error(transparent)]
    Transport(#[from] TransportFailure),
    #[error(transparent)]
    Build(#[from] DatadogError),
}

async fn post(
    logger: &DatadogLogger,
    body: Vec<u8>,
    compressed: bool,
) -> Result<TransportResponse, SendError> {
    let request = PostRequest {
        url: logger.settings.intake_url.clone(),
        headers: headers(logger, compressed)?,
        body,
        timeout: LOGGING_TIMEOUT,
    };
    let response = logger.transport.post(request).await?;
    if response.status >= 400 {
        return Err(SendError::Status {
            status: response.status,
            detail: detail(&response.body),
        });
    }
    Ok(response)
}

pub(crate) async fn async_send_compressed_data(
    logger: &DatadogLogger,
    data: &[DatadogPayload],
) -> Result<TransportResponse, SendError> {
    let encoded = safe_dumps(&data, DEFAULT_MAX_RECURSE_DEPTH, None).map_err(DatadogError::Json)?;
    let compressed = gzip(encoded.as_bytes())?;
    post(logger, compressed, true).await
}

pub(crate) async fn send_plain(
    logger: &DatadogLogger,
    payload: &DatadogPayload,
) -> Result<(), SendError> {
    let encoded =
        safe_dumps(payload, DEFAULT_MAX_RECURSE_DEPTH, None).map_err(DatadogError::Json)?;
    let response = post(logger, encoded.into_bytes(), false).await?;
    if response.status == 202 {
        return Ok(());
    }
    Err(SendError::Status {
        status: response.status,
        detail: detail(&response.body),
    })
}

fn headers(logger: &DatadogLogger, compressed: bool) -> Result<HeaderMap, SendError> {
    let mut headers = HeaderMap::new();
    if compressed {
        headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    }
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let Some(api_key) = &logger.settings.api_key else {
        return Ok(headers);
    };
    let mut value =
        HeaderValue::from_str(api_key.expose_secret()).map_err(DatadogError::ApiKeyHeader)?;
    value.set_sensitive(true);
    headers.insert(API_KEY_HEADER, value);
    Ok(headers)
}

fn detail(body: &[u8]) -> String {
    str::from_utf8(body)
        .unwrap_or("<non-UTF-8 body>")
        .to_owned()
}

pub(crate) fn gzip(data: &[u8]) -> Result<Vec<u8>, DatadogError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).map_err(DatadogError::Compress)?;
    encoder.finish().map_err(DatadogError::Compress)
}

pub fn exceeds_intake_limits(batch: &[DatadogPayload]) -> Result<bool, DatadogError> {
    if batch.len() > DD_MAX_BATCH_SIZE {
        return Ok(true);
    }
    let encoded =
        safe_dumps(&batch, DEFAULT_MAX_RECURSE_DEPTH, None).map_err(DatadogError::Json)?;
    Ok(encoded.len() > DD_MAX_PAYLOAD_SIZE_BYTES)
}

pub(crate) struct DatadogBatch<'logger> {
    pub(crate) logger: &'logger DatadogLogger,
}

impl BatchTransport<DatadogPayload> for DatadogBatch<'_> {
    type SendError = SendError;
    type LimitError = DatadogError;

    fn send(
        &self,
        batch: &[DatadogPayload],
    ) -> impl Future<Output = Result<BatchResponse, SendFailure<Self::SendError>>> + Send {
        let logger = self.logger;
        async move {
            match async_send_compressed_data(logger, batch).await {
                Ok(response) => Ok(BatchResponse {
                    status: response.status,
                    detail: detail(&response.body),
                }),
                Err(SendError::Status { status, detail }) => {
                    Err(SendFailure::Status { status, detail })
                }
                Err(error) => Err(SendFailure::Other(error)),
            }
        }
    }

    fn exceeds_limits(&self, batch: &[DatadogPayload]) -> Result<bool, Self::LimitError> {
        exceeds_intake_limits(batch)
    }
}

impl BatchDelivery<DatadogPayload> for DatadogBatch<'_> {
    async fn deliver(
        &self,
        batch: &[DatadogPayload],
        settled: &AtomicUsize,
    ) -> Result<Vec<DatadogPayload>, BatchCancelled<DatadogPayload>> {
        send_batch_with_413_split(
            self,
            batch,
            DeliverySettings {
                success_status_codes: &SUCCESS_STATUS_CODES,
                policy: RequeuePolicy::RequeueAll,
                integration_name: "Datadog",
                drop_error_message: DATADOG_413_ERROR,
            },
            settled,
        )
        .await
    }
}
