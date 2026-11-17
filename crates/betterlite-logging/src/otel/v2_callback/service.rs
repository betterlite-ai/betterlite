use std::collections::BTreeMap;
use std::time::SystemTime;

use opentelemetry::trace::SpanContext;
use opentelemetry_sdk::trace::Span as SdkSpan;

use crate::otel::emitter::span_emission::EmitOptions;
use crate::otel::mappers::base::SpanData;
use crate::otel::model::payloads::error::SpanError;
use crate::otel::model::payloads::service::{ServiceLoggerPayload, ServiceSpanData};
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::post_response::{
    ServiceSpanTiming, resolve_service_span_context,
};
use crate::otel::v2_callback::callback::V2Callback;
use crate::otel::vocabulary::resolvers::span_role_for_service;

#[derive(Clone, Copy)]
pub struct ServiceHook<'a> {
    pub payload: &'a ServiceLoggerPayload,
    pub parent_otel_span: Option<&'a SpanContext>,
    pub start_time: Option<SystemTime>,
    pub timing: ServiceSpanTiming,
    pub event_metadata: Option<&'a BTreeMap<String, OtelModelValue>>,
}

pub struct ServiceEmission<'hook, 'error> {
    pub hook: ServiceHook<'hook>,
    pub error_override: Option<&'error str>,
}

impl V2Callback {
    pub fn async_service_success_hook(&self, hook: &ServiceHook<'_>) {
        let emission = ServiceEmission {
            hook: *hook,
            error_override: None,
        };
        let _emitted = self.emit_service(&emission);
    }

    pub fn async_service_failure_hook(&self, hook: &ServiceHook<'_>, error: Option<&str>) {
        let override_text = match error {
            Some(text) if !text.is_empty() => text,
            Some(_) | None => "error",
        };
        let emission = ServiceEmission {
            hook: *hook,
            error_override: Some(override_text),
        };
        let _emitted = self.emit_service(&emission);
    }

    // litellm.integrations.otel.logger.OpenTelemetryV2._emit_service
    fn emit_service(&self, emission: &ServiceEmission<'_, '_>) -> Option<SdkSpan> {
        let metadata = emission.hook.event_metadata.cloned().unwrap_or_default();
        let mut data = ServiceSpanData::from_payload(emission.hook.payload, &metadata);
        let role = span_role_for_service(&data.service_name)?;
        if emission.error_override.is_none()
            && emission.hook.start_time.is_none()
            && emission.hook.timing.end_time.is_none()
            && emission.hook.parent_otel_span.is_none()
        {
            return None;
        }
        if let Some(text) = emission.error_override
            && data.error.is_none()
        {
            data.error = Some(SpanError {
                message: Some(text.to_owned()),
                ..SpanError::default()
            });
        }
        let (parent, links) =
            resolve_service_span_context(emission.hook.parent_otel_span, &emission.hook.timing);
        let options = EmitOptions {
            parent: Some(&parent),
            start_time: emission.hook.start_time,
            end_time: emission.hook.timing.end_time,
            tracer: None,
            links: &links,
        };
        self.logger
            .emitter
            .emit(role, &SpanData::Service(&data), &options)
    }
}
