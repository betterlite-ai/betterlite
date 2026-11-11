use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::langfuse_settings::Settings;
use betterlite_env::flag;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{
    BatchConfigBuilder, BatchSpanProcessor, SdkTracer, SdkTracerProvider, SpanExporter, SpanLimits,
};
use secrecy::SecretString;
use tokio::runtime::Handle;
use tokio::task::JoinSet;
use tokio::time::{Instant, timeout};

use crate::langfuse_exporter::discard::DiscardingSpanExporter;
use crate::langfuse_exporter::error::LangfuseExporterError;
use crate::langfuse_observations::requested_ids::RequestedIdGenerator;
use crate::langfuse_settings::{LangfuseSettingsError, debug};

mod cache;
pub mod exporter;
mod key;
mod setup;

use self::exporter::ExportLedger;
use self::key::{SampleRateKey, channel_spec, tracing_key};
use self::setup::{resource, sampler};

pub const TRACER_NAME: &str = "langfuse-sdk";
// litellm's _CHANNEL_RETIRE_GRACE_SECONDS = 60.0
pub const CHANNEL_RETIRE_GRACE: Duration = Duration::from_secs(60);
pub const MAX_QUEUE_SIZE: usize = 100_000;
pub const DEFAULT_FLUSH_TIMEOUT_MILLIS: u64 = 30_000;
pub const UNSET: u32 = u32::MAX;

// LiteLLM leaves the attribute-count and length ceilings at SpanLimits.UNSET; this SDK has no length limits
pub const SPAN_LIMITS: SpanLimits = SpanLimits {
    max_attributes_per_span: UNSET,
    max_events_per_span: 128,
    max_links_per_span: 128,
    max_attributes_per_event: 128,
    max_attributes_per_link: 128,
};

// litellm's _TracingKey: the credentials and export settings that tell one channel from another
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TracingKey {
    public_key: String,
    secret_key_digest: [u8; 32],
    base_url: String,
    environment: Option<String>,
    release: Option<String>,
    sample_rate: SampleRateKey,
    flush_at: usize,
    flush_interval_millis: u64,
    mock_mode: bool,
}

// One export channel per credential set: a provider, the tracer Langfuse keys spans by, and the ledger behind them
#[derive(Clone)]
pub struct LangfuseTracing {
    pub provider: SdkTracerProvider,
    pub tracer: SdkTracer,
    // litellm's _ExportLedger wraps the exporter and counts the batches it failed to deliver
    pub ledger: ExportLedger,
}

#[derive(Debug, thiserror::Error)]
pub enum LangfuseChannelError {
    #[error(transparent)]
    Settings(#[from] LangfuseSettingsError),
    #[error(transparent)]
    Exporter(#[from] LangfuseExporterError),
}

#[derive(Clone, Copy)]
pub struct ChannelSpec<'spec> {
    pub environment: Option<&'spec str>,
    pub release: Option<&'spec str>,
    pub sample_rate: f64,
    pub flush_interval_millis: u64,
    pub flush_at: usize,
}

pub struct LangfuseChannelRequest {
    pub public_key: String,
    pub secret_key: SecretString,
    pub base_url: String,
    pub environment: Option<String>,
    pub release: Option<String>,
    pub flush_interval: u64,
    pub mock_mode: bool,
}

// litellm's _FlushWorker thread: one flusher per channel, all waited on to one deadline
struct FlushWorker {
    channel: LangfuseTracing,
    timeout_millis: u64,
}

impl LangfuseTracing {
    #[must_use]
    pub async fn flush(&self, timeout_millis: u64) -> bool {
        let failed_before = self.ledger.failed_batches.load(Ordering::Acquire);
        let provider = self.provider.clone();
        let mut drain = JoinSet::new();
        drain.spawn_blocking(move || provider.force_flush());
        let joined = timeout(Duration::from_millis(timeout_millis), drain.join_next()).await;
        let drained = matches!(joined, Ok(Some(Ok(Ok(())))));
        drained && self.ledger.failed_batches.load(Ordering::Acquire) == failed_before
    }

    pub fn shutdown(&self) {
        if let Err(error) = self.provider.shutdown() {
            tracing::debug!(%error, "the langfuse export channel did not shut its batch processor down");
        }
    }
}

#[must_use]
pub fn build_langfuse_tracing<E>(
    exporter: E,
    runtime: &Handle,
    spec: &ChannelSpec<'_>,
    lookup: &dyn EnvironmentLookup,
) -> LangfuseTracing
where
    E: SpanExporter + 'static,
{
    let disabled = otel_sdk_disabled(lookup);
    let resource = resource(spec.environment, spec.release);
    let mut prepared = exporter;
    prepared.set_resource(&resource);
    let ledger = ExportLedger {
        exporter: Arc::new(prepared),
        failed_batches: Arc::default(),
        runtime: runtime.clone(),
    };
    let config = BatchConfigBuilder::default()
        .with_max_queue_size(MAX_QUEUE_SIZE)
        .with_max_export_batch_size(spec.flush_at)
        .with_scheduled_delay(Duration::from_millis(spec.flush_interval_millis))
        .build();
    let provider = SdkTracerProvider::builder()
        .with_resource(resource)
        .with_sampler(sampler(spec.sample_rate, disabled))
        .with_id_generator(RequestedIdGenerator::default())
        .with_span_limits(SPAN_LIMITS)
        .with_span_processor(
            BatchSpanProcessor::builder(ledger.clone())
                .with_batch_config(config)
                .build(),
        )
        .build();
    let tracer = provider.tracer(TRACER_NAME);
    LangfuseTracing {
        provider,
        tracer,
        ledger,
    }
}

fn otel_sdk_disabled(lookup: &dyn EnvironmentLookup) -> bool {
    let Some(raw) = lookup.lookup("OTEL_SDK_DISABLED") else {
        return false;
    };
    match flag("OTEL_SDK_DISABLED", &raw) {
        // LiteLLM writes this through verbose_logger; betterlite emits it on its tracing subscriber
        Ok(Some(true)) => {
            tracing::warn!(
                "OTEL_SDK_DISABLED=true also disables the langfuse callback's export channel"
            );
            true
        }
        Ok(Some(false) | None) => false,
        Err(error) => {
            tracing::warn!(%error, "OTEL_SDK_DISABLED is not a bool; ignoring it");
            false
        }
    }
}

pub fn enable_langfuse_debug_logging() {
    if tracing::enabled!(tracing::Level::DEBUG) {
        tracing::debug!("LANGFUSE_DEBUG is set and the log filter includes langfuse debug output");
    } else {
        tracing::info!(
            "LANGFUSE_DEBUG is set; set the log filter to include debug output to see it"
        );
    }
}

pub fn acquire_langfuse_tracing<E, F>(
    settings: &Settings,
    lookup: &dyn EnvironmentLookup,
    runtime: &Handle,
    request: &LangfuseChannelRequest,
    live_exporter: F,
) -> Result<LangfuseTracing, LangfuseChannelError>
where
    E: SpanExporter + 'static,
    F: FnOnce() -> Result<E, LangfuseExporterError>,
{
    if debug(settings)? {
        enable_langfuse_debug_logging();
    }
    let spec = channel_spec(settings, request);
    let key = tracing_key(request, &spec);
    let mut state = cache::TRACING
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(lease) = state.leases.get_mut(&key) {
        if let Some(retire) = lease.retire.take() {
            retire.task.abort();
        }
        lease.holders = lease.holders.saturating_add(1);
        return Ok(lease.tracing.clone());
    }
    let tracing = if request.mock_mode {
        build_langfuse_tracing(DiscardingSpanExporter, runtime, &spec, lookup)
    } else {
        build_langfuse_tracing(live_exporter()?, runtime, &spec, lookup)
    };
    cache::insert(&mut state, key, &tracing, runtime);
    drop(state);
    Ok(tracing)
}

pub async fn release_langfuse_tracing(tracing: &LangfuseTracing, grace: Duration) {
    if let cache::ReleaseOutcome::Shutdown(released) = cache::begin_release(tracing, grace) {
        let mut shutdown = JoinSet::new();
        shutdown.spawn_blocking(move || released.shutdown());
        let _finished = shutdown.join_next().await;
    }
}

#[must_use]
pub async fn flush_langfuse_tracing(timeout_millis: u64) -> bool {
    let channels: Vec<LangfuseTracing> = cache::channels();
    let window = Duration::from_millis(timeout_millis);
    let started = Instant::now();
    let mut workers = JoinSet::new();
    for channel in channels {
        let worker = FlushWorker {
            channel,
            timeout_millis,
        };
        workers.spawn(async move { worker.channel.flush(worker.timeout_millis).await });
    }
    let mut flushed = true;
    loop {
        let remaining = window.saturating_sub(started.elapsed());
        match timeout(remaining, workers.join_next()).await {
            Err(_elapsed) => return false,
            Ok(None) => return flushed,
            Ok(Some(Ok(worker_flushed))) => flushed &= worker_flushed,
            Ok(Some(Err(_stopped))) => return false,
        }
    }
}

#[path = "langfuse_channels/tests/delivery.rs"]
mod delivery;
#[path = "langfuse_channels/tests/support.rs"]
mod support;
#[path = "langfuse_channels/tests.rs"]
mod tests;
