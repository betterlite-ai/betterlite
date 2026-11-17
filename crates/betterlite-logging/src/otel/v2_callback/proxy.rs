use crate::logging_metadata::exception::ExceptionFacts;
use crate::otel::emitter::error_stamping::ErrorStamp;
use crate::otel::plumbing::context::context_slots::{AnchoredSpan, RequestTraceContext};
use crate::otel::v2_callback::callback::V2Callback;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::v2_callback::span_error::span_error_from_exception;
use crate::otel::vocabulary::constants::attribute_keys::error as error_keys;

impl V2Callback {
    // litellm.integrations.otel.logger.OpenTelemetryV2.create_litellm_proxy_request_started_span
    pub fn mark_request_root_span(
        &self,
        span: &LiveSpan,
        http_route: Option<&str>,
        trace: &mut RequestTraceContext,
    ) {
        // set_request_root_span no-ops for a span whose context is not valid, as is_recordable_span reads it
        if !span.context.is_valid() {
            return;
        }
        self.roots.register(span);
        let route = http_route
            .map(str::to_owned)
            .or_else(|| trace.root.as_ref().and_then(|root| root.http_route.clone()));
        trace.anchor_request_root(AnchoredSpan {
            context: span.context.clone(),
            http_route: route,
        });
    }

    // litellm.integrations.otel.plumbing.context.request_root_span, read through the handle mark recorded
    #[must_use]
    pub fn request_root_span(&self, trace: &RequestTraceContext) -> Option<LiveSpan> {
        let root = trace.root.as_ref()?;
        let id = root.context.span_id();
        self.roots.get(id)
    }

    // litellm.integrations.otel.logger.OpenTelemetryV2.record_error_attributes_on_span
    pub fn record_error_attributes_on_span(
        &self,
        span: Option<&LiveSpan>,
        exception: Option<&ExceptionFacts>,
        status_code: u16,
    ) {
        let Some(span) = span else {
            return;
        };
        if !span.is_recording() {
            return;
        }
        let error = span_error_from_exception(&self.error_info, exception, Some(status_code), None);
        let stamp = ErrorStamp {
            record_event: !span.has_attribute(error_keys::TYPE),
            set_status: true,
        };
        span.stamp(&error, stamp);
    }

    // litellm.integrations.otel.logger.OpenTelemetryV2.async_post_call_failure_hook
    pub fn async_post_call_failure_hook(
        &self,
        exception: Option<&ExceptionFacts>,
        traceback_str: Option<&str>,
        parent_span: Option<&LiveSpan>,
        trace: &RequestTraceContext,
    ) {
        let transport = trace
            .mcp_message_transport_span()
            .and_then(|context| self.roots.get(context.span_id()));
        let root = self.request_root_span(trace);
        let span = transport.or(root).or_else(|| parent_span.cloned());
        let Some(span) = span else {
            return;
        };
        if !span.is_recording() {
            return;
        }
        let error = span_error_from_exception(&self.error_info, exception, None, traceback_str);
        span.stamp(&error, ErrorStamp::FULL);
    }
}
