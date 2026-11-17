use opentelemetry::trace::{Span as _, TraceContextExt as _};
use opentelemetry::{Context, ContextGuard};

use crate::otel::emitter::error_stamping::{ErrorStamp, stamp_error};
use crate::otel::emitter::span_emission::StartOptions;
use crate::otel::model::payloads::error::SpanError;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::v2_callback::callback::V2Callback;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::vocabulary::registry::SpanRole;

// the phase's live span; dropping it ends the span, as the with-statement's finally does
pub struct PhaseSpan {
    pub span: LiveSpan,
}

// litellm.integrations.otel.logger.OpenTelemetryV2.start_phase_span
#[must_use]
pub fn start_phase_span(
    callback: &V2Callback,
    name: &str,
    trace: &RequestTraceContext,
) -> PhaseSpan {
    let parent = trace.resolve_request_span_context();
    let options = StartOptions {
        parent: Some(&parent),
        start_time: None,
        tracer: None,
        links: &[],
    };
    let span = callback
        .logger
        .emitter
        .start_span(SpanRole::Service, name, &options);
    PhaseSpan {
        span: LiveSpan::from(span),
    }
}

impl PhaseSpan {
    // LiteLLM's use_span(span, end_on_exit=True): the runtime hook (B8910) holds this guard while the phase runs
    #[must_use]
    pub fn attach(&self) -> ContextGuard {
        Context::current()
            .with_remote_span_context(self.span.context.clone())
            .attach()
    }

    // use_span records the exception and sets ERROR before LiteLLM's attribute-only stamp runs
    pub fn record_error(&self, error: &SpanError) {
        let mut span = self.span.lock();
        if span.is_recording() {
            let _stamped = stamp_error(&mut *span, error, ErrorStamp::FULL);
        }
    }
}

impl Drop for PhaseSpan {
    fn drop(&mut self) {
        self.span.lock().end();
    }
}
