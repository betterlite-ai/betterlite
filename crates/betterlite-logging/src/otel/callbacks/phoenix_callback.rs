use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use opentelemetry::Context;
use opentelemetry::propagation::TextMapPropagator as _;
use opentelemetry::trace::{Span as _, SpanKind, Status, TraceContextExt as _, Tracer as _};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracer;
use secrecy::ExposeSecret as _;

use crate::otel::config::{ARIZE_PROJECT_NAME, PHOENIX_PROJECT_NAME};
use crate::otel::integrations::phoenix_settings::{PhoenixConfig, PhoenixSettingsError};
use crate::otel::legacy_exporters::tracer_cache::project::tracer_for;
use crate::otel::legacy_lifecycle::error::LegacyLifecycleError;
use crate::otel::legacy_lifecycle::lifecycle::{
    LegacyLifecycle, LegacyLifecycleOptions, LifecycleCall, LifecycleEvent, litellm_params,
};
use crate::otel::legacy_lifecycle::tracer::{TracerRouter, tracer_for_request};
use crate::otel::legacy_lifecycle::{context, parent, recording};
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig, Signals};
use crate::otel::legacy_policy::span_time::to_ns;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::parent_span::PARENT_SPAN_KEY;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel_provider_assembly::legacy_otel_init::LegacyOtel;

mod tests;

pub const CALLBACK_NAME: &str = "arize_phoenix";
pub const PROXY_PARENT_SPAN_NAME: &str = "litellm_proxy_request";
pub const DEFAULT_PROJECT: &str = "default";
pub const HEALTHY_MESSAGE: &str = "Arize-Phoenix credentials are configured properly";
pub const MISSING_API_KEY_MESSAGE: &str = "PHOENIX_API_KEY environment variable not set";

#[must_use]
pub fn project_attributes(project: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("model_id".to_owned(), project.to_owned()),
        ("openinference.project.name".to_owned(), project.to_owned()),
        ("service.name".to_owned(), project.to_owned()),
    ])
}

// _is_proxy_request reads only the server-set proxy_server_request, never client metadata
#[must_use]
pub fn is_proxy_request(kwargs: &MetadataMap) -> bool {
    litellm_params(kwargs)
        .and_then(|params| params.get("proxy_server_request"))
        .and_then(OtelModelValue::as_object)
        .is_some_and(|request| !request.is_empty())
}

// _resolve_project_name: metadata, then the environment, then "default"
#[must_use]
pub fn resolve_project_name(
    kwargs: &MetadataMap,
    environment: &BTreeMap<String, String>,
) -> String {
    let proxy_mode = is_proxy_request(kwargs);
    ["phoenix_project_name_override", "phoenix_project_name"]
        .iter()
        .find_map(|key| metadata_project(kwargs, key, proxy_mode))
        .or_else(|| env_project_name(environment))
        .unwrap_or_else(|| DEFAULT_PROJECT.to_owned())
}

// _metadata_project_from_kwargs reads standard_logging_object before litellm_params
fn metadata_project(kwargs: &MetadataMap, key: &str, proxy_mode: bool) -> Option<String> {
    ["standard_logging_object", "litellm_params"]
        .iter()
        .filter_map(|bucket| {
            kwargs
                .get(*bucket)
                .and_then(OtelModelValue::as_object)
                .and_then(|fields| fields.get("metadata"))
                .and_then(OtelModelValue::as_object)
        })
        .find_map(|metadata| {
            if let Some(auth) = metadata
                .get("user_api_key_auth_metadata")
                .and_then(OtelModelValue::as_object)
                && let Some(project) = auth.get(key).and_then(OtelModelValue::as_str)
                && !project.trim().is_empty()
            {
                return Some(project.trim().to_owned());
            }
            if proxy_mode {
                return None;
            }
            let text = metadata.get(key)?.as_str()?.trim();
            (!text.is_empty()).then(|| text.to_owned())
        })
}

fn env_project_name(environment: &BTreeMap<String, String>) -> Option<String> {
    [PHOENIX_PROJECT_NAME, ARIZE_PROJECT_NAME]
        .iter()
        .find_map(|name| {
            let value = environment.get(*name)?.trim();
            (!value.is_empty()).then(|| value.to_owned())
        })
}

// every span of one request goes to the tracer of its resolved project
pub struct PhoenixProjects {
    environment: BTreeMap<String, String>,
}

impl TracerRouter for PhoenixProjects {
    fn route(
        &self,
        lifecycle: &LegacyLifecycle,
        kwargs: &MetadataMap,
    ) -> Result<Option<SdkTracer>, LegacyLifecycleError> {
        let project = resolve_project_name(kwargs, &self.environment);
        let attributes = project_attributes(&project);
        let tracer = tracer_for(&lifecycle.tracer_cache, &project, &attributes)?;
        Ok(Some(tracer))
    }
}

// ArizePhoenixLogger's per-request trace: a project tracer, a local proxy parent and Phoenix's own context
pub fn handle_trace(
    lifecycle: &LegacyLifecycle,
    call: &LifecycleCall<'_>,
    event: LifecycleEvent,
) -> Result<(), LegacyLifecycleError> {
    let mut kwargs = call.kwargs.clone();
    strip_parent(&mut kwargs);
    let parent = is_proxy_request(&kwargs)
        .then(|| proxy_parent(lifecycle, &kwargs, call))
        .transpose()?;
    if let Some(open_parent) = &parent {
        carry_parent(&mut kwargs, open_parent);
    }
    let traced = LifecycleCall {
        kwargs: &kwargs,
        response: call.response,
        start_time: call.start_time,
        end_time: call.end_time,
        exception: call.exception,
        flags: call.flags,
    };
    let result = lifecycle.handle(&traced, event);
    if let Some(open_parent) = parent {
        finish_parent(lifecycle, &open_parent, &traced, event);
    }
    result
}

// the base provider's parent lives on another provider, so Phoenix ignores every litellm_parent_otel_span
fn strip_parent(kwargs: &mut MetadataMap) {
    let Some(OtelModelValue::Object(params)) = kwargs.get_mut("litellm_params") else {
        return;
    };
    for bucket in ["metadata", "litellm_metadata"] {
        if let Some(OtelModelValue::Object(fields)) = params.get_mut(bucket) {
            drop(fields.remove(PARENT_SPAN_KEY));
        }
    }
}

// _get_phoenix_context creates its own SERVER parent for a proxied request, under the incoming traceparent
fn proxy_parent(
    lifecycle: &LegacyLifecycle,
    kwargs: &MetadataMap,
    call: &LifecycleCall<'_>,
) -> Result<LiveSpan, LegacyLifecycleError> {
    let tracer = tracer_for_request(lifecycle, kwargs)?;
    let builder = tracer
        .span_builder(PROXY_PARENT_SPAN_NAME)
        .with_kind(SpanKind::Server)
        .with_start_time(context::instant_from_ns(to_ns(
            call.start_time,
            lifecycle.now_ns(),
        )));
    let incoming = parent::from_kwargs(kwargs)?.context.unwrap_or_default();
    Ok(LiveSpan::from(
        tracer.build_with_context(builder, &incoming),
    ))
}

fn carry_parent(kwargs: &mut MetadataMap, parent: &LiveSpan) {
    let mut carrier = HashMap::new();
    TraceContextPropagator::new().inject_context(
        &Context::new().with_remote_span_context(parent.context.clone()),
        &mut carrier,
    );
    let Some(traceparent) = carrier.remove("traceparent") else {
        return;
    };
    let Some(OtelModelValue::Object(params)) = kwargs.get_mut("litellm_params") else {
        return;
    };
    let metadata = params
        .entry("metadata".to_owned())
        .or_insert_with(|| OtelModelValue::Object(MetadataMap::default()));
    if let OtelModelValue::Object(fields) = metadata {
        fields.insert(
            PARENT_SPAN_KEY.to_owned(),
            OtelModelValue::String(traceparent),
        );
    }
}

// Phoenix closes its parent after the request span and the guardrails, carrying the same status and attributes
fn finish_parent(
    lifecycle: &LegacyLifecycle,
    parent: &LiveSpan,
    call: &LifecycleCall<'_>,
    event: LifecycleEvent,
) {
    let status = match event {
        LifecycleEvent::Success => Status::Ok,
        LifecycleEvent::Failure => {
            recording::record_exception(parent, call);
            Status::error("")
        }
    };
    parent.lock().set_status(status);
    let capture = lifecycle.capture_policy.resolve(call.flags);
    recording::apply_attributes(lifecycle, parent, call, capture);
    parent
        .lock()
        .end_with_timestamp(context::instant_from_ns(to_ns(
            call.end_time,
            lifecycle.now_ns(),
        )));
}

// the registration config litellm_logging builds for the arize_phoenix integration
pub fn otel_config(
    environment: &BTreeMap<String, String>,
) -> Result<OpenTelemetryConfig, PhoenixSettingsError> {
    let phoenix = PhoenixConfig::from_env(environment)?;
    let mut config = OpenTelemetryConfig::from_env(environment);
    config.exporter = Exporter::Named(phoenix.protocol.exporter_kind().to_owned());
    config.endpoint = Some(phoenix.endpoint);
    config.headers = phoenix
        .otlp_auth_headers
        .map(|header| header.expose_secret().to_owned());
    // litellm_logging builds this config by hand, so the environment's signal switches leave both off
    config.signals = Signals {
        enable_metrics: false,
        enable_events: false,
    };
    // ArizePhoenixLogger._init_tracing builds private per-project providers instead of the global one
    config.skip_set_global = true;
    config.normalize(environment);
    Ok(config)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum PhoenixHealth {
    Healthy { message: &'static str },
    Unhealthy { error_message: String },
}

// async_health_check does not catch get_arize_phoenix_config's cloud error, so it reaches the caller
pub fn health_check(
    environment: &dyn EnvironmentLookup,
) -> Result<PhoenixHealth, PhoenixSettingsError> {
    let config = PhoenixConfig::from_env(environment)?;
    if config.otlp_auth_headers.is_none() {
        return Ok(PhoenixHealth::Unhealthy {
            error_message: MISSING_API_KEY_MESSAGE.to_owned(),
        });
    }
    Ok(PhoenixHealth::Healthy {
        message: HEALTHY_MESSAGE,
    })
}

#[must_use]
pub fn phoenix_lifecycle(
    otel: LegacyOtel,
    mut options: LegacyLifecycleOptions,
    environment: &BTreeMap<String, String>,
) -> LegacyLifecycle {
    options.use_request_span = true;
    let mut lifecycle = LegacyLifecycle::from_otel(otel, options);
    lifecycle.tracer_router = Some(Arc::new(PhoenixProjects {
        environment: environment.clone(),
    }));
    lifecycle
}
