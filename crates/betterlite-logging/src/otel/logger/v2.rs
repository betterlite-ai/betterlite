use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use indexmap::IndexMap;
use opentelemetry::Context;
use opentelemetry::baggage::{Baggage, BaggageExt as _};
use opentelemetry::trace::{Link, TraceContextExt as _};
use opentelemetry_sdk::trace::SdkTracer;

use crate::otel::config::Settings;
use crate::otel::emitter::span_emission::SpanEmitter;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::plumbing::metrics::recorder::GenAiMetricRecorder;
use crate::otel::plumbing::routing::cache::{TenantProvider, TenantTracerCache};

// LiteLLM's _OPEN_CALLS_MAX: a span opened at pre_call that never reaches a callback would linger, so the oldest carrier is evicted past this many
pub const OPEN_CALLS_MAX: usize = 10_000;

// litellm.integrations.otel.logger._LLMCallSpan: the state carried from the pre_call boundary to close; a boundary is present when pre_call saw a recordable parent, else creation is deferred to the close callback
pub struct OpenCall {
    // the span itself is created at close: the SDK ends and exports any span it drops, while LiteLLM's abandoned carrier never ends
    pub boundary: Option<BoundarySpan>,
    pub start_time: SystemTime,
    // the routed provider the boundary route leased, released exactly once when the carrier goes
    pub provider: Option<Arc<TenantProvider>>,
}

// what pre_call captured for the boundary span: the request parent, the routed tracer and the detached links back to the request trace
pub struct BoundarySpan {
    pub parent: Context,
    pub tracer: SdkTracer,
    pub links: Vec<Link>,
}

// litellm.integrations.otel.logger's _open_llm_calls ordered map
#[derive(Default)]
pub struct OpenCalls {
    pub calls: IndexMap<String, OpenCall>,
}

impl OpenCalls {
    // idempotent storage keyed by call id; an evicted carrier comes back so its provider lease is released
    #[must_use]
    pub fn store(&mut self, call_id: String, call: OpenCall) -> Option<OpenCall> {
        self.calls.insert(call_id, call);
        if self.calls.len() > OPEN_CALLS_MAX {
            let (_, evicted) = self.calls.shift_remove_index(0)?;
            return Some(evicted);
        }
        None
    }
}

// the request's anchored trace state, injected by the proxy; a task with no request bound sees the empty context, which makes pre_call defer
pub trait RequestTraceSource: Send + Sync {
    fn trace_context(&self) -> RequestTraceContext;
}

// a link back to the request trace, for a span detached into its own trace
#[must_use]
pub fn request_trace_links(context: &Context) -> Vec<Link> {
    let span = context.span();
    let anchor = span.span_context();
    if anchor.is_valid() {
        vec![Link::with_context(anchor.clone())]
    } else {
        Vec::new()
    }
}

// what a detached span starts from: the request's Baggage with no parent span, so the routed span roots its own trace
#[must_use]
pub fn detached_context(context: &Context) -> Context {
    let baggage: Baggage = context
        .baggage()
        .into_iter()
        .map(|(key, entry)| (key.clone(), entry.clone()))
        .collect();
    Context::new().with_baggage(baggage)
}

// what building the logger takes: the assembled provider and emitter, the metric recorder, the tenant cache and the request scope
pub struct OpenTelemetryV2Parts {
    pub name: String,
    pub config: Settings,
    pub tracer: SdkTracer,
    pub emitter: SpanEmitter<SdkTracer>,
    pub metrics: Option<Arc<GenAiMetricRecorder>>,
    pub tenants: TenantTracerCache,
    pub trace: Arc<dyn RequestTraceSource>,
}

// litellm.integrations.otel.logger.OpenTelemetryV2: the inference lifecycle and tenant routing around the span engine
pub struct OpenTelemetryV2 {
    pub(crate) name: String,
    pub(crate) config: Settings,
    pub(crate) tracer: SdkTracer,
    pub(crate) emitter: SpanEmitter<SdkTracer>,
    pub(crate) metrics: Option<Arc<GenAiMetricRecorder>>,
    pub(crate) metric_filter_error_logged: AtomicBool,
    pub(crate) tenants: TenantTracerCache,
    pub(crate) open_calls: Mutex<OpenCalls>,
    pub(crate) trace: Arc<dyn RequestTraceSource>,
}

impl OpenTelemetryV2 {
    #[must_use]
    pub fn with_parts(parts: OpenTelemetryV2Parts) -> Self {
        Self {
            name: parts.name,
            config: parts.config,
            tracer: parts.tracer,
            emitter: parts.emitter,
            metrics: parts.metrics,
            metric_filter_error_logged: AtomicBool::new(false),
            tenants: parts.tenants,
            open_calls: Mutex::new(OpenCalls::default()),
            trace: parts.trace,
        }
    }

    pub(crate) fn open_calls(&self) -> MutexGuard<'_, OpenCalls> {
        match self.open_calls.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    // LiteLLM's _store_open_call: remember an in-flight call, releasing the provider of an evicted carrier
    pub(crate) fn store_open_call(&self, call_id: String, call: OpenCall) {
        let evicted = { self.open_calls().store(call_id, call) };
        if let Some(carrier) = evicted.as_ref() {
            self.tenants.release(carrier.provider.as_ref());
        }
    }
}
