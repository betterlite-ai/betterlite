use std::borrow::Cow;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_logging::langfuse_observations::requested_ids::requested_ids;
use opentelemetry::trace::{
    Span, SpanBuilder, SpanContext, SpanId, Status, TraceContextExt as _, TraceFlags, TraceId,
    TraceState, Tracer,
};
use opentelemetry::{Context, KeyValue};

pub(crate) struct ScriptedSpan {
    pub(crate) context: SpanContext,
    pub(crate) parent: SpanContext,
    pub(crate) name: String,
    pub(crate) start_time: Option<SystemTime>,
    pub(crate) attributes: Vec<KeyValue>,
    pub(crate) requested_trace_id: Option<TraceId>,
    pub(crate) requested_span_id: Option<SpanId>,
    pub(crate) end_time: Option<SystemTime>,
}

impl Span for ScriptedSpan {
    fn add_event_with_timestamp<T>(
        &mut self,
        _name: T,
        _timestamp: SystemTime,
        _attributes: Vec<KeyValue>,
    ) where
        T: Into<Cow<'static, str>>,
    {
    }

    fn span_context(&self) -> &SpanContext {
        &self.context
    }

    fn is_recording(&self) -> bool {
        false
    }

    fn set_attribute(&mut self, _attribute: KeyValue) {}

    fn set_status(&mut self, _status: Status) {}

    fn update_name<T>(&mut self, _new_name: T)
    where
        T: Into<Cow<'static, str>>,
    {
    }

    fn add_link(&mut self, _span_context: SpanContext, _attributes: Vec<KeyValue>) {}

    fn end(&mut self) {}

    fn end_with_timestamp(&mut self, timestamp: SystemTime) {
        self.end_time = Some(timestamp);
    }
}

pub(crate) struct ScriptedTracer {
    pub(crate) span_id: SpanId,
}

impl Tracer for ScriptedTracer {
    type Span = ScriptedSpan;

    fn build_with_context(&self, builder: SpanBuilder, parent_cx: &Context) -> Self::Span {
        let (requested_trace_id, requested_span_id) = requested_ids();
        ScriptedSpan {
            context: SpanContext::new(
                TraceId::INVALID,
                self.span_id,
                TraceFlags::SAMPLED,
                false,
                TraceState::NONE,
            ),
            parent: parent_cx.span().span_context().clone(),
            name: builder.name.to_string(),
            start_time: builder.start_time,
            attributes: builder.attributes.unwrap_or_default(),
            requested_trace_id,
            requested_span_id,
            end_time: None,
        }
    }
}

pub(crate) fn system_nanos(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_nanos()).ok(),
        Err(_error) => None,
    }
}
