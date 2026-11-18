use opentelemetry::Context;
use opentelemetry::trace::{Span as _, Status};

use crate::otel::legacy_policy::capture::CaptureMode;
use crate::otel::legacy_policy::dedupe::ScopeValue;
use crate::otel::legacy_policy::span_time::to_ns;
use crate::otel::plumbing::context::parent_span::ParentSpan;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::vocabulary::span_names::PROXY_REQUEST_SPAN_NAME;

use super::error::LegacyLifecycleError;
use super::lifecycle::{self, LegacyLifecycle, LifecycleCall, resolve_guardrail_context};

impl LegacyLifecycle {
    pub fn handle_failure(&self, call: &LifecycleCall<'_>) -> Result<(), LegacyLifecycleError> {
        // unlike the success path there is no proxy side effect to preserve on the skip path
        if !self.emit_once(call.kwargs, &[ScopeValue::Text("failure".to_owned())]) {
            return Ok(());
        }
        let parent = self.parent(call.kwargs)?;
        let live_parent = parent
            .span
            .as_ref()
            .and_then(|context| self.live.get(context.span_id()));
        let capture_mode = self.capture_policy.resolve(call.flags);
        let parent_context = parent.context.clone().unwrap_or_default();
        let primary = self.failure_primary(call, &parent, &parent_context, capture_mode)?;
        let guardrail_context = resolve_guardrail_context(
            primary.as_ref(),
            live_parent.as_ref(),
            parent.context.as_ref(),
        );
        self.create_guardrail_span(call.kwargs, &guardrail_context)?;
        // only the active span is parent_otel_span here; a metadata carrier answers None upstream
        if let Some(span) = &live_parent
            && lifecycle::name_of(span).as_deref() == Some(PROXY_REQUEST_SPAN_NAME)
        {
            // no is_recording guard here: the failure hook ends the already ended span again
            span.lock()
                .end_with_timestamp(lifecycle::instant_from_ns(to_ns(
                    call.end_time,
                    self.now_ns(),
                )));
        }
        Ok(())
    }

    fn failure_primary(
        &self,
        call: &LifecycleCall<'_>,
        parent: &ParentSpan,
        parent_context: &Context,
        capture_mode: CaptureMode,
    ) -> Result<Option<LiveSpan>, LegacyLifecycleError> {
        if parent.span.is_none() || self.use_request_span {
            let tracer = self.tracer_for_request(call.kwargs)?;
            let span = self.start_primary_span(&tracer, call, parent_context);
            span.lock().set_status(Status::error(""));
            self.apply_attributes(&span, call, capture_mode);
            self.record_exception(&span, call);
            span.lock()
                .end_with_timestamp(lifecycle::instant_from_ns(to_ns(
                    call.end_time,
                    self.now_ns(),
                )));
            return Ok(Some(span));
        }
        // the shallow path records the failure on the parent span itself
        if let Some(span) = parent
            .span
            .as_ref()
            .and_then(|context| self.live.get(context.span_id()))
        {
            let recording = span.lock().is_recording();
            if recording {
                span.lock().set_status(Status::error(""));
                self.apply_attributes(&span, call, capture_mode);
                self.record_exception(&span, call);
            }
        }
        Ok(None)
    }
}
