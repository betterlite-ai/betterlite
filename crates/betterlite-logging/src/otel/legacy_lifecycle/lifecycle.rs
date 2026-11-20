use std::sync::Arc;
use std::time::UNIX_EPOCH;

use opentelemetry::Context;
use opentelemetry::trace::TraceContextExt as _;
use opentelemetry_sdk::trace::SdkTracer;

use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::otel::legacy_exporters::tracer_cache::DynamicTracerCache;
use crate::otel::legacy_metrics::events::SemanticEventEmitter;
use crate::otel::legacy_metrics::recorder::LegacyMetricRecorder;
use crate::otel::legacy_policy::capture::{CapturePolicy, LoggingFlags};
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::legacy_policy::dedupe::ScopeValue;
use crate::otel::legacy_policy::error_recording::RecordedException;
use crate::otel::legacy_policy::span_time::SpanTime;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::events::{Clock, SystemClock};
use crate::otel::v2_callback::live_span::{LiveSpan, RootSpans};
use crate::otel_provider_assembly::legacy_otel_init::LegacyOtel;

use super::context::resolve_guardrail_context;
use super::error::LegacyLifecycleError;
use super::state::{RequestDedupe, request_key};
use super::tracer::{BaseDynamicOtel, DynamicOtel, TracerChoice, TracerRouter};
use super::{context, failure, guardrail, proxy, success};

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent {
    Success,
    Failure,
}

// a vendor's sampling decision runs before the lifecycle's dedupe, where LiteLLM's hooks return early
pub trait ExportGate: Send + Sync {
    fn should_export(
        &self,
        kwargs: &MetadataMap,
        event: LifecycleEvent,
    ) -> Result<bool, LegacyLifecycleError>;
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
    pub export_gate: Option<Arc<dyn ExportGate>>,
    pub tracer_router: Option<Arc<dyn TracerRouter>>,
}

impl LegacyLifecycle {
    pub fn handle(
        &self,
        call: &LifecycleCall<'_>,
        event: LifecycleEvent,
    ) -> Result<(), LegacyLifecycleError> {
        if let Some(gate) = &self.export_gate
            && !gate.should_export(call.kwargs, event)?
        {
            return Ok(());
        }
        let scope = match event {
            LifecycleEvent::Success => "success",
            LifecycleEvent::Failure => "failure",
        };
        if !self.dedupe.emit_once(
            request_key(call.kwargs).as_deref(),
            &[ScopeValue::Text(scope.to_owned())],
        ) {
            // a repeated success still closes its proxy; a repeated failure has no side effect
            if event == LifecycleEvent::Success {
                proxy::end_proxy_from_kwargs(self, call);
            }
            return Ok(());
        }
        let spans = context::request_spans(self, call, event)?;
        let guardrail_context = resolve_guardrail_context(
            spans.primary.as_ref(),
            spans.parent.as_ref(),
            spans.fallback.as_ref(),
        );
        guardrail::create_guardrail_span(self, call.kwargs, &guardrail_context)?;
        match event {
            LifecycleEvent::Success => success::finish_success(self, call, &spans),
            LifecycleEvent::Failure => {
                failure::close_proxy(self, call, spans.parent.as_ref());
                Ok(())
            }
        }
    }

    pub fn emit_request_guardrails(
        &self,
        request_data: &MetadataMap,
        parent: Option<&LiveSpan>,
    ) -> Result<(), LegacyLifecycleError> {
        let bucket = if request_data.contains_key("litellm_metadata") {
            "litellm_metadata"
        } else {
            "metadata"
        };
        let metadata = request_data
            .get(bucket)
            .and_then(OtelModelValue::as_object)
            .cloned()
            .unwrap_or_default();
        let Some(information) = metadata.get("standard_logging_guardrail_information") else {
            return Ok(());
        };
        // the same bucket reaches the failure handler, so the entry markers share the request key
        let standard_logging_object = MetadataMap::from([
            ("guardrail_information".to_owned(), information.clone()),
            (
                "metadata".to_owned(),
                OtelModelValue::Object(metadata.clone()),
            ),
        ]);
        let litellm_params =
            MetadataMap::from([("metadata".to_owned(), OtelModelValue::Object(metadata))]);
        let kwargs = MetadataMap::from([
            (
                "litellm_params".to_owned(),
                OtelModelValue::Object(litellm_params),
            ),
            (
                "standard_logging_object".to_owned(),
                OtelModelValue::Object(standard_logging_object),
            ),
        ]);
        let context = parent.map_or_else(Context::new, |span| {
            Context::new().with_remote_span_context(span.context.clone())
        });
        guardrail::create_guardrail_span(self, &kwargs, &context)
    }

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
            export_gate: None,
            tracer_router: None,
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

    #[must_use]
    pub fn now_ns(&self) -> i64 {
        self.clock
            .now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
            })
    }
}
