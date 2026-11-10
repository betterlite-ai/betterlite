use std::fmt;
use std::slice;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::langfuse_settings::Settings;
use http::{HeaderMap, HeaderName};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use url::Url;

use crate::langfuse_exporter::SERVER_FLOOR_HINT;
use crate::langfuse_exporter::encoder::{OtlpSpanEncoder, SpanEncoder};
use crate::langfuse_exporter::endpoint::{endpoint, request_headers};
use crate::langfuse_exporter::error::LangfuseExporterError;
use crate::langfuse_exporter::options::{ExporterOptions, exporter_options};
use crate::langfuse_exporter::retry::{DeliveryOutcome, retry_delays};
use crate::langfuse_exporter::sleep::{Sleep, TokioSleep};
use crate::langfuse_exporter::transport::{HttpRequest, Transport};
use crate::langfuse_exporter::truncation::truncated;
use crate::langfuse_settings::Credentials;

pub struct LangfuseSpanExporter<T> {
    endpoint: Url,
    headers: HeaderMap,
    timeout: Duration,
    delays: Vec<Duration>,
    encoder: Arc<dyn SpanEncoder>,
    transport: T,
    sleep: Arc<dyn Sleep>,
    resource: Resource,
}

impl<T> fmt::Debug for LangfuseSpanExporter<T>
where
    T: Transport,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The Authorization header carries the key pair, so Debug names the headers and never prints a value
        let names: Vec<&str> = self.headers.keys().map(HeaderName::as_str).collect();
        f.debug_struct("LangfuseSpanExporter")
            .field("endpoint", &self.endpoint.as_str())
            .field("header_names", &names)
            .field("timeout", &self.timeout)
            .field("delays", &self.delays)
            .finish_non_exhaustive()
    }
}

pub fn build_span_exporter<T>(
    settings: &Settings,
    environment: &dyn EnvironmentLookup,
    credentials: &Credentials,
    transport: T,
) -> Result<LangfuseSpanExporter<T>, LangfuseExporterError>
where
    T: Transport,
{
    LangfuseSpanExporter::new(
        &exporter_options(settings, environment, credentials)?,
        transport,
        Arc::new(OtlpSpanEncoder),
        Arc::new(TokioSleep),
    )
}

impl<T> SpanExporter for LangfuseSpanExporter<T>
where
    T: Transport,
{
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        if self.export_rounds(batch).await {
            return Ok(());
        }
        Err(OTelSdkError::InternalFailure(
            "the langfuse export did not deliver every span".to_owned(),
        ))
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        Ok(())
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.resource = resource.clone();
    }
}

impl<T> LangfuseSpanExporter<T>
where
    T: Transport,
{
    pub fn new(
        options: &ExporterOptions,
        transport: T,
        encoder: Arc<dyn SpanEncoder>,
        sleep: Arc<dyn Sleep>,
    ) -> Result<Self, LangfuseExporterError> {
        let requested = endpoint(&options.base_url, &options.export_path);
        let endpoint =
            Url::parse(&requested).map_err(|source| LangfuseExporterError::Endpoint {
                endpoint: requested,
                source,
            })?;
        let headers = request_headers(options.public_key.as_deref(), options.secret_key.as_ref())?;
        Ok(Self {
            endpoint,
            headers,
            timeout: options.timeout,
            delays: retry_delays(options.retries),
            encoder,
            transport,
            sleep,
            resource: Resource::builder_empty().build(),
        })
    }

    async fn export_rounds(&self, batch: Vec<SpanData>) -> bool {
        let mut pending = vec![batch];
        let mut delivered = true;
        while !pending.is_empty() {
            let (next, round_delivered) = self.settle_round(pending).await;
            delivered &= round_delivered;
            // Halving an empty batch cannot shrink a body, so empty halves are never sent
            pending = next.into_iter().filter(|half| !half.is_empty()).collect();
        }
        delivered
    }

    async fn settle_round(&self, pending: Vec<Vec<SpanData>>) -> (Vec<Vec<SpanData>>, bool) {
        let mut next = Vec::new();
        let mut delivered = true;
        for batch in pending {
            match self.send_batch(&batch).await {
                DeliveryOutcome::Delivered => {}
                DeliveryOutcome::Rejected => delivered = false,
                DeliveryOutcome::TooLarge => next.extend(smaller(&batch)),
            }
        }
        (next, delivered)
    }

    async fn send_batch(&self, batch: &[SpanData]) -> DeliveryOutcome {
        let Some(encoded) = self.encode(batch) else {
            return DeliveryOutcome::Rejected;
        };
        let payload: Arc<[u8]> = Arc::from(encoded);
        let outcome = self.deliver(&payload).await;
        if outcome != DeliveryOutcome::TooLarge {
            return outcome;
        }
        match batch {
            [only] if truncated(only).is_none() => {
                tracing::error!(
                    bytes = payload.len(),
                    "langfuse rejected a span as too large and nothing is left to truncate; dropping it"
                );
                DeliveryOutcome::Rejected
            }
            _ => {
                tracing::warn!(
                    spans = batch.len(),
                    "langfuse rejected the batch as too large; resending it smaller"
                );
                DeliveryOutcome::TooLarge
            }
        }
    }

    async fn deliver(&self, body: &Arc<[u8]>) -> DeliveryOutcome {
        let mut last = self.post(body).await;
        for delay in &self.delays {
            let Some(settled) = last else {
                tracing::warn!(delay = delay.as_secs(), "langfuse export failed; retrying");
                self.sleep.sleep(*delay).await;
                last = self.post(body).await;
                continue;
            };
            return settled;
        }
        last.unwrap_or_else(|| {
            tracing::error!(
                retries = self.delays.len(),
                "langfuse export failed after every retry"
            );
            DeliveryOutcome::Rejected
        })
    }

    async fn post(&self, body: &Arc<[u8]>) -> Option<DeliveryOutcome> {
        let request = HttpRequest {
            url: self.endpoint.clone(),
            headers: self.headers.clone(),
            body: Arc::clone(body),
            timeout: self.timeout,
        };
        match self.transport.send(request).await {
            Ok(response) => status_outcome(response.status),
            Err(error) if T::retryable(&error) => {
                tracing::warn!(%error, "langfuse export failed in the transport");
                None
            }
            Err(error) => {
                tracing::error!(%error, "langfuse export failed in the transport and is not retried");
                Some(DeliveryOutcome::Rejected)
            }
        }
    }

    fn encode(&self, batch: &[SpanData]) -> Option<Vec<u8>> {
        if let Some(body) = self.encoder.encode(batch, &self.resource) {
            return Some(body);
        }
        let kept: Vec<SpanData> = batch
            .iter()
            .filter(|span| {
                self.encoder
                    .encode(slice::from_ref(*span), &self.resource)
                    .is_some()
            })
            .cloned()
            .collect();
        tracing::error!(
            dropped = batch.len().saturating_sub(kept.len()),
            "langfuse dropped the spans the OTLP encoder rejected"
        );
        if kept.is_empty() {
            return None;
        }
        self.encoder.encode(&kept, &self.resource)
    }
}

// None means the request is worth sending again; 413 asks for a smaller body and every other non-2xx is final, as httpx raise_for_status makes it
fn status_outcome(status: u16) -> Option<DeliveryOutcome> {
    match status {
        408 | 429 | 500..=599 => {
            tracing::warn!(
                status,
                "langfuse export was answered with a status worth retrying"
            );
            None
        }
        413 => Some(DeliveryOutcome::TooLarge),
        200..=299 => Some(DeliveryOutcome::Delivered),
        _ => {
            // LiteLLM points a 404 at the self-hosted server floor the OTLP route needs
            let hint = if status == 404 { SERVER_FLOOR_HINT } else { "" };
            tracing::error!(status, "langfuse rejected the export{hint}");
            Some(DeliveryOutcome::Rejected)
        }
    }
}

fn smaller(batch: &[SpanData]) -> Vec<Vec<SpanData>> {
    match batch {
        [only] => truncated(only).map_or_else(Vec::new, |span| vec![vec![span]]),
        _ => batch
            .split_at_checked(batch.len().div_euclid(2))
            .map_or_else(Vec::new, |(first, second)| {
                vec![first.to_vec(), second.to_vec()]
            }),
    }
}
