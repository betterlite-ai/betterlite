use opentelemetry::Context;
use opentelemetry::trace::{Span as _, Status, TraceContextExt as _, Tracer as _};

use crate::otel::legacy_attributes::raw_request::set_raw_request_attributes;
use crate::otel::legacy_policy::capture::CaptureMode;
use crate::otel::legacy_policy::dedupe::ScopeValue;
use crate::otel::legacy_policy::semconv_attrs::semconv_latest_experimental;
use crate::otel::legacy_policy::span_time::to_ns;
use crate::otel::legacy_policy::text_field;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::parent_span::ParentSpan;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::vocabulary::span_names::PROXY_REQUEST_SPAN_NAME;

use super::error::LegacyLifecycleError;
use super::lifecycle::{
    self, LegacyLifecycle, LifecycleCall, litellm_params, resolve_guardrail_context,
};

pub const RAW_REQUEST_SPAN_NAME: &str = "raw_gen_ai_request";

impl LegacyLifecycle {
    pub fn maybe_log_raw_request(
        &self,
        call: &LifecycleCall<'_>,
        parent_context: &Context,
    ) -> Result<(), LegacyLifecycleError> {
        // the raw payload span carries provider-specific keys, which the stable semconv replaces
        if semconv_latest_experimental(&self.config) {
            return Ok(());
        }
        if !self.capture_policy.resolve(call.flags).capture_in_span() {
            return Ok(());
        }
        let generation_name = litellm_params(call.kwargs)
            .and_then(|params| params.get("metadata"))
            .and_then(OtelModelValue::as_object)
            .and_then(|metadata| text_field(metadata, "generation_name"));
        let name = generation_name
            .map(str::to_owned)
            .unwrap_or_else(|| RAW_REQUEST_SPAN_NAME.to_owned());
        let tracer = self.tracer_for_request(call.kwargs)?;
        let builder = tracer
            .span_builder(name)
            .with_start_time(lifecycle::instant_from_ns(to_ns(
                call.start_time,
                self.now_ns(),
            )));
        let mut span = tracer.build_with_context(builder, parent_context);
        span.set_status(Status::Ok);
        set_raw_request_attributes(&mut span, call.kwargs);
        self.write_team_attributes_from_std_log(&mut span, call.kwargs);
        span.end_with_timestamp(lifecycle::instant_from_ns(to_ns(
            call.end_time,
            self.now_ns(),
        )));
        Ok(())
    }

    pub fn handle_success(&self, call: &LifecycleCall<'_>) -> Result<(), LegacyLifecycleError> {
        // the sync and async callbacks both fire for a stream, and the skip path still closes the proxy span
        if !self.emit_once(call.kwargs, &[ScopeValue::Text("success".to_owned())]) {
            self.end_proxy_from_kwargs(call);
            return Ok(());
        }
        let parent = self.parent(call.kwargs)?;
        let live_parent = parent
            .span
            .as_ref()
            .and_then(|context| self.live.get(context.span_id()));
        let capture_mode = self.capture_policy.resolve(call.flags);
        let parent_context = parent.context.clone().unwrap_or_default();
        let primary = self.success_primary(call, &parent, &parent_context, capture_mode)?;
        self.finish_success(
            call,
            primary.as_ref(),
            live_parent.as_ref(),
            parent.context.as_ref(),
        )
    }

    fn success_primary(
        &self,
        call: &LifecycleCall<'_>,
        parent: &ParentSpan,
        parent_context: &Context,
        capture_mode: CaptureMode,
    ) -> Result<Option<LiveSpan>, LegacyLifecycleError> {
        if parent.span.is_none() || self.use_request_span {
            let tracer = self.tracer_for_request(call.kwargs)?;
            let span = self.start_primary_span(&tracer, call, parent_context);
            span.lock().set_status(Status::Ok);
            self.apply_attributes(&span, call, capture_mode);
            span.lock()
                .end_with_timestamp(lifecycle::instant_from_ns(to_ns(
                    call.end_time,
                    self.now_ns(),
                )));
            let raw_context = Context::new().with_remote_span_context(span.context.clone());
            self.maybe_log_raw_request(call, &raw_context)?;
            return Ok(Some(span));
        }
        // a parent span keeps the hierarchy shallow: its own span carries the request attributes
        if let Some(span) = parent
            .span
            .as_ref()
            .and_then(|context| self.live.get(context.span_id()))
        {
            span.lock().set_status(Status::Ok);
            self.apply_attributes(&span, call, capture_mode);
        }
        self.maybe_log_raw_request(call, parent_context)?;
        Ok(None)
    }

    fn finish_success(
        &self,
        call: &LifecycleCall<'_>,
        primary: Option<&LiveSpan>,
        live_parent: Option<&LiveSpan>,
        fallback: Option<&Context>,
    ) -> Result<(), LegacyLifecycleError> {
        let guardrail_context = resolve_guardrail_context(primary, live_parent, fallback);
        self.create_guardrail_span(call.kwargs, &guardrail_context)?;
        self.record_metrics(call)?;
        if let Some(span) = primary.or(live_parent) {
            self.emit_semantic_logs(call, &span.context)?;
        }
        if let Some(span) = live_parent {
            let recording = span.lock().is_recording();
            if lifecycle::name_of(span).as_deref() == Some(PROXY_REQUEST_SPAN_NAME) && recording {
                self.close_proxy_ok(span, call.end_time);
            }
        }
        self.stamp_proxy_team(call.kwargs);
        self.end_proxy_from_kwargs(call);
        Ok(())
    }
}
