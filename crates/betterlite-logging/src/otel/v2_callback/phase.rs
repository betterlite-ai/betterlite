use crate::otel::emitter::error_stamping::ErrorStamp;
use crate::otel::emitter::span_emission::StartOptions;
use crate::otel::model::payloads::error::SpanError;
use crate::otel::v2_callback::callback::V2Callback;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::vocabulary::registry::SpanRole;

// the phase's live span; dropping it ends the span, as the with-statement's finally does
pub struct PhaseSpan {
    pub span: LiveSpan,
}

impl V2Callback {
    // litellm.integrations.otel.logger.OpenTelemetryV2.start_phase_span
    #[must_use]
    pub fn start_phase_span(&self, name: &str) -> PhaseSpan {
        let options = StartOptions {
            parent: None,
            start_time: None,
            tracer: None,
            links: &[],
        };
        let span = self
            .logger
            .emitter
            .start_span(SpanRole::Service, name, &options);
        PhaseSpan {
            span: LiveSpan::from(span),
        }
    }
}

impl PhaseSpan {
    // use_span records the exception and sets ERROR before LiteLLM's attribute-only stamp runs
    pub fn record_error(&self, error: &SpanError) {
        if self.span.is_recording() {
            self.span.stamp(error, ErrorStamp::FULL);
        }
    }
}

impl Drop for PhaseSpan {
    fn drop(&mut self) {
        self.span.end();
    }
}
