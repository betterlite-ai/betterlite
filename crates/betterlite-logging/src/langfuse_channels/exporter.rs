use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use tokio::runtime::Handle;

// The SDK exporter trait returns an opaque future, so the channel cache holds exporters behind this view
pub(super) trait SharedExporter: fmt::Debug + Send + Sync {
    fn export<'export>(
        &'export self,
        batch: Vec<SpanData>,
    ) -> Pin<Box<dyn Future<Output = OTelSdkResult> + Send + 'export>>;

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult;

    fn force_flush(&self) -> OTelSdkResult;
}

impl<E> SharedExporter for E
where
    E: SpanExporter,
{
    fn export<'export>(
        &'export self,
        batch: Vec<SpanData>,
    ) -> Pin<Box<dyn Future<Output = OTelSdkResult> + Send + 'export>> {
        Box::pin(SpanExporter::export(self, batch))
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        SpanExporter::shutdown_with_timeout(self, timeout)
    }

    fn force_flush(&self) -> OTelSdkResult {
        SpanExporter::force_flush(self)
    }
}

// litellm's _ExportLedger routes the batch processor's exports and counts the failed ones
#[derive(Clone, Debug)]
pub struct ExportLedger {
    pub(super) exporter: Arc<dyn SharedExporter>,
    pub(super) failed_batches: Arc<AtomicUsize>,
    pub(super) runtime: Handle,
}

impl SpanExporter for ExportLedger {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let exporter = Arc::clone(&self.exporter);
        let dispatched = self
            .runtime
            .spawn(async move { exporter.export(batch).await });
        let result = match dispatched.await {
            Ok(result) => result,
            Err(source) => Err(OTelSdkError::InternalFailure(format!(
                "the langfuse export task stopped before it reported: {source}"
            ))),
        };
        if result.is_err() {
            self.failed_batches.fetch_add(1, Ordering::AcqRel);
        }
        result
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.exporter.shutdown_with_timeout(timeout)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.exporter.force_flush()
    }

    // The resource is set on the exporter before it is shared, because set_resource needs &mut
    fn set_resource(&mut self, _resource: &Resource) {}
}
