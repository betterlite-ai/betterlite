use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opentelemetry::Context;
use opentelemetry::trace::{SpanKind, TraceContextExt as _, Tracer as _};
use opentelemetry_sdk::trace::SdkTracer;

use crate::exporter_options::failure_counter::{CallbackFailureCounter, handle_callback_failure};
use crate::otel::legacy_exporters::tracer_cache::DynamicTracerCache;
use crate::otel::legacy_metrics::events::SemanticEventEmitter;
use crate::otel::legacy_metrics::recorder::LegacyMetricRecorder;
use crate::otel::legacy_policy::capture::{CapturePolicy, LoggingFlags};
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::legacy_policy::error_recording::RecordedException;
use crate::otel::legacy_policy::semconv_attrs::{
    gen_ai_operation_name, semconv_latest_experimental,
};
use crate::otel::legacy_policy::span_time::{SpanTime, span_name, to_ns};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::parent_span::ParentSpan;
use crate::otel::plumbing::events::{Clock, SystemClock};
use crate::otel::v2_callback::live_span::{LiveSpan, RootSpans};
use crate::otel_provider_assembly::legacy_otel_init::LegacyOtel;

use super::error::LegacyLifecycleError;
use super::state::RequestDedupe;
use super::tracer::{BaseDynamicOtel, DynamicOtel, TracerChoice};

pub const DEFAULT_CALLBACK_NAME: &str = "opentelemetry";
pub const DEFAULT_HANDLER_CLASS_NAME: &str = "OpenTelemetry";

// one lifecycle invocation: LiteLLM's (kwargs, response_obj, start_time, end_time) plus the raised exception
pub struct LifecycleCall<'call> {
    pub kwargs: &'call MetadataMap,
    pub response: Option<&'call OtelModelValue>,
    pub start_time: SpanTime,
    pub end_time: SpanTime,
    pub exception: Option<&'call RecordedException>,
    pub flags: LoggingFlags,
}

#[must_use]
pub fn litellm_params(kwargs: &MetadataMap) -> Option<&MetadataMap> {
    kwargs
        .get("litellm_params")
        .and_then(OtelModelValue::as_object)
}

pub struct LegacyLifecycleOptions {
    pub dynamic: Arc<dyn DynamicOtel>,
    pub live: Arc<RootSpans>,
    pub metrics: Option<LegacyMetricRecorder>,
    pub events: Option<SemanticEventEmitter>,
    pub failure_counters: Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
    pub handler_class_name: String,
    pub use_request_span: bool,
    pub clock: Box<dyn Clock>,
}

impl Default for LegacyLifecycleOptions {
    fn default() -> Self {
        Self {
            dynamic: Arc::new(BaseDynamicOtel),
            live: Arc::new(RootSpans::default()),
            metrics: None,
            events: None,
            failure_counters: Vec::new(),
            handler_class_name: DEFAULT_HANDLER_CLASS_NAME.to_owned(),
            use_request_span: false,
            clock: Box::new(SystemClock),
        }
    }
}

pub struct LegacyLifecycle {
    pub config: OpenTelemetryConfig,
    pub callback_name: Option<String>,
    pub tracer: SdkTracer,
    pub tracer_cache: DynamicTracerCache,
    pub capture_policy: CapturePolicy,
    pub live: Arc<RootSpans>,
    pub dynamic: Arc<dyn DynamicOtel>,
    pub metrics: Option<LegacyMetricRecorder>,
    pub events: Option<SemanticEventEmitter>,
    pub failure_counters: Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
    pub dedupe: RequestDedupe,
    pub clock: Box<dyn Clock>,
    pub use_request_span: bool,
}

impl LegacyLifecycle {
    #[must_use]
    pub fn from_otel(otel: LegacyOtel, options: LegacyLifecycleOptions) -> Self {
        Self {
            config: otel.config,
            callback_name: otel.callback_name,
            tracer: otel.tracer,
            tracer_cache: otel.tracer_cache,
            capture_policy: otel.capture_policy,
            live: options.live,
            dynamic: options.dynamic,
            metrics: options.metrics,
            events: options.events,
            failure_counters: options.failure_counters,
            dedupe: RequestDedupe::new(options.handler_class_name),
            clock: options.clock,
            use_request_span: options.use_request_span,
        }
    }

    pub fn tracer_choice(
        &self,
        kwargs: &MetadataMap,
    ) -> Result<TracerChoice, LegacyLifecycleError> {
        super::tracer::tracer_choice(
            &self.config,
            self.callback_name.as_deref(),
            self.dynamic.as_ref(),
            kwargs,
        )
    }

    pub(super) fn count_failure(&self) {
        let mut counters: Vec<&dyn CallbackFailureCounter> =
            Vec::with_capacity(self.failure_counters.len());
        for counter in &self.failure_counters {
            counters.push(counter.as_ref());
        }
        handle_callback_failure(
            self.callback_name
                .as_deref()
                .unwrap_or(DEFAULT_CALLBACK_NAME),
            &counters,
        );
    }

    #[must_use]
    pub fn now_ns(&self) -> i64 {
        match self.clock.now().duration_since(UNIX_EPOCH) {
            Ok(duration) => i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX),
            Err(_) => 0,
        }
    }

    pub(super) fn parent(&self, kwargs: &MetadataMap) -> Result<ParentSpan, LegacyLifecycleError> {
        if self.config.ignore_context_propagation {
            return Ok(ParentSpan::default());
        }
        Ok(super::parent::from_kwargs(kwargs)?)
    }

    pub(super) fn start_primary_span(
        &self,
        tracer: &SdkTracer,
        call: &LifecycleCall<'_>,
        context: &Context,
    ) -> LiveSpan {
        let params = litellm_params(call.kwargs).cloned().unwrap_or_default();
        let semconv = semconv_latest_experimental(&self.config);
        let operation = gen_ai_operation_name(
            call.kwargs
                .get("call_type")
                .and_then(OtelModelValue::as_str),
        );
        let model = call.kwargs.get("model").and_then(OtelModelValue::as_str);
        let name = span_name(&params, operation, model, semconv);
        let mut builder = tracer
            .span_builder(name)
            .with_start_time(instant_from_ns(to_ns(call.start_time, self.now_ns())));
        if semconv {
            builder = builder.with_kind(SpanKind::Client);
        }
        LiveSpan::from(tracer.build_with_context(builder, context))
    }
}

// an ended SDK span reports no data, so only a recording span answers with its name
#[must_use]
pub fn name_of(span: &LiveSpan) -> Option<String> {
    if let Some(data) = span.lock().exported_data() {
        return Some(data.name.to_string());
    }
    None
}

// the clock source is whole nanoseconds, and a pre-epoch value still maps to a SystemTime
#[must_use]
pub fn instant_from_ns(nanoseconds: i64) -> SystemTime {
    let magnitude = Duration::from_nanos(nanoseconds.unsigned_abs());
    if nanoseconds.is_negative() {
        UNIX_EPOCH.checked_sub(magnitude).unwrap_or(UNIX_EPOCH)
    } else {
        UNIX_EPOCH.checked_add(magnitude).unwrap_or(UNIX_EPOCH)
    }
}

// a guardrail is parented to the span the request landed on, never to an orphaned root
#[must_use]
pub fn resolve_guardrail_context(
    primary: Option<&LiveSpan>,
    parent_span: Option<&LiveSpan>,
    fallback: Option<&Context>,
) -> Context {
    if let Some(span) = primary.or(parent_span) {
        return Context::new().with_remote_span_context(span.context.clone());
    }
    fallback.cloned().unwrap_or_default()
}

#[must_use]
pub fn standard_logging_object(kwargs: &MetadataMap) -> Option<&MetadataMap> {
    kwargs
        .get("standard_logging_object")
        .and_then(OtelModelValue::as_object)
}
