use std::io::Write as _;
use std::slice::from_ref;
use std::time::Duration;

use flate2::Compression;
use flate2::write::GzEncoder;
use http::header::{CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;

use crate::batch::retry::is_retryable_status;

use super::NewRelicMetricsError;
use super::aggregation::build_metric_payload;
use super::logger::NewRelicMetricsLogger;
use super::queue::QueuedRecord;
use super::records::NewRelicMetricRecord;
use super::transport::NewRelicPost;
use super::wire::NewRelicMetricEnvelope;

const API_KEY_HEADER: HeaderName = HeaderName::from_static("api-key");
const CLIENT_ERROR_LOW: u16 = 400;
const CLIENT_ERROR_HIGH: u16 = 500;
const SUCCESS_LOW: u16 = 200;
const SUCCESS_HIGH: u16 = 300;
const FORBIDDEN: u16 = 403;

pub(super) async fn classify_and_send(
    logger: &NewRelicMetricsLogger,
    batch: &[QueuedRecord],
    window_start: Duration,
) -> bool {
    let records: Vec<NewRelicMetricRecord> =
        batch.iter().map(|queued| queued.record.clone()).collect();
    let Some(request) = compressed_request(logger, &records, window_start) else {
        return true;
    };
    match logger.transport.post(request).await {
        Ok(status) => settled_by_status(status, records.len()),
        Err(error) => {
            tracing::warn!(
                %error,
                records = records.len(),
                "New Relic Metrics: network error sending records, will retry"
            );
            false
        }
    }
}

// A record that cannot be aggregated or encoded fails the same way on every retry, so it is dropped
fn compressed_request(
    logger: &NewRelicMetricsLogger,
    records: &[NewRelicMetricRecord],
    window_start: Duration,
) -> Option<NewRelicPost> {
    let payload = match build_metric_payload(records, window_start, (logger.clock)()) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::error!(
                %error,
                records = records.len(),
                "New Relic Metrics: the metric payload could not be built; dropping the batch"
            );
            return None;
        }
    };
    match compressed_post(logger, &payload) {
        Ok(post) => Some(post),
        Err(error) => {
            tracing::error!(
                %error,
                records = records.len(),
                "New Relic Metrics: the metric payload could not be encoded; dropping the batch"
            );
            None
        }
    }
}

fn compressed_post(
    logger: &NewRelicMetricsLogger,
    payload: &NewRelicMetricEnvelope,
) -> Result<NewRelicPost, NewRelicMetricsError> {
    let json = serde_json::to_vec(from_ref(payload)).map_err(NewRelicMetricsError::Encode)?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&json)
        .map_err(NewRelicMetricsError::Compress)?;
    let body = encoder.finish().map_err(NewRelicMetricsError::Compress)?;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    let mut api_key = HeaderValue::try_from(logger.api_key.expose_secret())
        .map_err(NewRelicMetricsError::ApiKeyHeader)?;
    api_key.set_sensitive(true);
    headers.insert(API_KEY_HEADER, api_key);
    Ok(NewRelicPost {
        url: logger.metric_api_url,
        headers,
        body,
    })
}

fn settled_by_status(status: u16, records: usize) -> bool {
    if (SUCCESS_LOW..SUCCESS_HIGH).contains(&status) {
        return true;
    }
    if (CLIENT_ERROR_LOW..CLIENT_ERROR_HIGH).contains(&status) && !is_retryable_status(status) {
        if status == FORBIDDEN {
            tracing::warn!(
                status,
                records,
                "New Relic Metrics: 403 from Metric API (permanent credential failure: invalid or revoked team ingest key), dropping records"
            );
        } else {
            tracing::warn!(
                status,
                records,
                "New Relic Metrics: client error from Metric API, dropping records"
            );
        }
        return true;
    }
    tracing::warn!(
        status,
        records,
        "New Relic Metrics: retryable status from Metric API, will retry records"
    );
    false
}
