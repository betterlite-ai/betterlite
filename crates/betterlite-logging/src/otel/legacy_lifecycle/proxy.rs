use opentelemetry::trace::{Span as _, Status};

use crate::otel::legacy_policy::span_time::{SpanTime, to_ns};
use crate::otel::logger_runtime::legacy::set_response_status_code;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::plumbing::context::parent_span::span_context_from_value;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::vocabulary::span_names::PROXY_REQUEST_SPAN_NAME;

use super::lifecycle::{self, LegacyLifecycle, LifecycleCall};
use super::parent;

impl LegacyLifecycle {
    fn proxy_span(&self, kwargs: &MetadataMap, with_litellm_metadata: bool) -> Option<LiveSpan> {
        parent::proxy_carrier(kwargs, with_litellm_metadata)
            .and_then(span_context_from_value)
            .and_then(|context| self.live.get(context.span_id()))
    }

    pub(super) fn stamp_proxy_team(&self, kwargs: &MetadataMap) {
        let Some(span) = self.proxy_span(kwargs, false) else {
            return;
        };
        if lifecycle::name_of(&span).as_deref() != Some(PROXY_REQUEST_SPAN_NAME) {
            return;
        }
        if span.lock().is_recording() {
            self.write_team_attributes_from_std_log(&mut *span.lock(), kwargs);
        }
    }

    pub(super) fn end_proxy_from_kwargs(&self, call: &LifecycleCall<'_>) {
        let Some(span) = self.proxy_span(call.kwargs, true) else {
            return;
        };
        if lifecycle::name_of(&span).as_deref() != Some(PROXY_REQUEST_SPAN_NAME) {
            return;
        }
        if span.lock().is_recording() {
            self.close_proxy_ok(&span, call.end_time);
        }
    }

    pub(super) fn close_proxy_ok(&self, span: &LiveSpan, end_time: SpanTime) {
        set_response_status_code(Some(span), Some(200));
        span.lock().set_status(Status::Ok);
        span.lock()
            .end_with_timestamp(lifecycle::instant_from_ns(to_ns(end_time, self.now_ns())));
    }
}
