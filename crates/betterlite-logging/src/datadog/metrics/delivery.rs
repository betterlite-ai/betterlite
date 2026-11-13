use std::sync::atomic::{AtomicUsize, Ordering};

use http::header::{CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;

use crate::batch::delivery::{BatchCancelled, BatchDelivery};
use crate::datadog_logging::send::gzip;
use crate::datadog_logging::transport::{LOGGING_TIMEOUT, PostRequest, TransportResponse};

use super::datadog_metrics::DatadogMetricsLogger;
use super::error::{MetricsError, SendError};
use super::series::{DatadogMetricSeries, DatadogMetricsPayload};

const API_KEY_HEADER: HeaderName = HeaderName::from_static("dd-api-key");
const APPLICATION_KEY_HEADER: HeaderName = HeaderName::from_static("dd-application-key");

pub(crate) async fn upload(
    logger: &DatadogMetricsLogger,
    series: &[DatadogMetricSeries],
) -> Result<(), SendError> {
    if logger.settings.api_key.is_none() {
        // LiteLLM returns without an exception here, so the batch is treated as delivered
        return Ok(());
    }
    let payload = DatadogMetricsPayload {
        series: series.to_vec(),
    };
    let payload_json = serde_json::to_vec(&payload).map_err(MetricsError::Encode)?;
    let request = PostRequest {
        url: logger.settings.upload_url.clone(),
        headers: headers(logger)?,
        body: gzip(&payload_json).map_err(MetricsError::Gzip)?,
        timeout: LOGGING_TIMEOUT,
    };
    let response: TransportResponse = logger.transport.post(request).await?;
    if !(200..=299).contains(&response.status) {
        return Err(SendError::Status {
            status: response.status,
        });
    }
    tracing::debug!(
        points = series.len(),
        status = response.status,
        "Datadog Metrics: uploaded metric points"
    );
    Ok(())
}

fn headers(logger: &DatadogMetricsLogger) -> Result<HeaderMap, MetricsError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    let Some(api_key) = &logger.settings.api_key else {
        return Ok(headers);
    };
    let mut value =
        HeaderValue::from_str(api_key.expose_secret()).map_err(MetricsError::ApiKeyHeader)?;
    value.set_sensitive(true);
    headers.insert(API_KEY_HEADER, value);
    if let Some(app_key) = &logger.settings.app_key {
        let mut app_value =
            HeaderValue::from_str(app_key.expose_secret()).map_err(MetricsError::ApiKeyHeader)?;
        app_value.set_sensitive(true);
        headers.insert(APPLICATION_KEY_HEADER, app_value);
    }
    Ok(headers)
}

pub(crate) struct DatadogMetricsBatch<'logger> {
    pub(crate) logger: &'logger DatadogMetricsLogger,
}

impl BatchDelivery<DatadogMetricSeries> for DatadogMetricsBatch<'_> {
    async fn deliver(
        &self,
        batch: &[DatadogMetricSeries],
        settled: &AtomicUsize,
    ) -> Result<Vec<DatadogMetricSeries>, BatchCancelled<DatadogMetricSeries>> {
        match upload(self.logger, batch).await {
            Ok(()) => {
                settled.fetch_add(batch.len(), Ordering::SeqCst);
                Ok(Vec::new())
            }
            // CustomBatchLogger keeps every event of a failed send for the next flush
            Err(error) => {
                tracing::error!(
                    %error,
                    points = batch.len(),
                    "Datadog Metrics: Error in async_send_batch"
                );
                Ok(batch.to_vec())
            }
        }
    }
}
