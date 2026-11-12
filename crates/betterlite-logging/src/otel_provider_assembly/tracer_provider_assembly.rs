use std::sync::Arc;

use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{SdkTracerProvider, TracerProviderBuilder};

use crate::otel::config::destination::SpanScope;
use crate::otel::config::{ExporterOwner, ExporterSpec, Settings};
use crate::otel::fan_out::attach::{SinkKey, sink_key};
use crate::otel::fan_out::overridden::OverriddenBackendFilter;
use crate::otel::fan_out::scope::RequestScopeSource;
use crate::otel::plumbing::exporters::{ExportResult, ExporterFactories, Transport};
use crate::otel::plumbing::headers::parse_headers;
use crate::otel::plumbing::processors::{BaggageSpanProcessor, processor_for};
use crate::otel::plumbing::sinks::SpanSink;

const SERVICE_NAME: &str = "service.name";
const DEPLOYMENT_ENVIRONMENT: &str = "deployment.environment";

// The explicit exporter and baggage processor are overrides for tests, as in LiteLLM.
#[derive(Default)]
pub struct AssemblyOptions {
    pub exporter: Option<SpanSink>,
    pub baggage_processor: Option<BaggageSpanProcessor>,
    pub use_simple_processor: Option<bool>,
    pub tenant_overrides: bool,
}

const fn owner_name(owner: ExporterOwner) -> &'static str {
    match owner {
        ExporterOwner::ArizeAx => "arize",
        ExporterOwner::ArizePhoenix => "arize_phoenix",
        ExporterOwner::LangfuseOtel => "langfuse_otel",
        ExporterOwner::WeaveOtel => "weave_otel",
        ExporterOwner::Levo => "levo",
        ExporterOwner::Agentops => "agentops",
        ExporterOwner::Newrelic => "newrelic",
        ExporterOwner::Signoz => "signoz",
    }
}

#[must_use]
pub fn provider_resource(config: &Settings) -> Resource {
    let mut attributes = vec![KeyValue::new(SERVICE_NAME, config.service_name.clone())];
    if let Some(environment) = config
        .deployment_environment
        .as_ref()
        .filter(|environment| !environment.is_empty())
    {
        attributes.push(KeyValue::new(DEPLOYMENT_ENVIRONMENT, environment.clone()));
    }
    attributes.extend(
        config
            .resource_attributes
            .iter()
            .map(|(name, value)| KeyValue::new(name.clone(), value.clone())),
    );
    Resource::builder().with_attributes(attributes).build()
}

// A spec the loop skipped for missing headers never reaches here, so only the transport decides.
fn wire_sink_key(spec: &ExporterSpec, factories: &ExporterFactories) -> Option<SinkKey> {
    if factories.exporter_transport(&spec.kind) == Transport::Headerless {
        return None;
    }
    sink_key(
        spec.endpoint.as_deref(),
        &parse_headers(spec.headers.as_deref()),
    )
}

struct Assembly<'assembly> {
    config: &'assembly Settings,
    factories: &'assembly ExporterFactories,
    scope_source: &'assembly Arc<dyn RequestScopeSource>,
    options: AssemblyOptions,
}

impl Assembly<'_> {
    fn add_exporter(
        &self,
        builder: TracerProviderBuilder,
        spec: &ExporterSpec,
    ) -> ExportResult<TracerProviderBuilder> {
        let sink = self.factories.exporter_from_spec(spec)?;
        let use_simple = spec
            .use_simple_processor
            .or(self.options.use_simple_processor);
        let processor = processor_for(sink, use_simple);
        let owner = spec
            .owner
            .filter(|_owner| self.options.tenant_overrides)
            .map(|owner| owner_name(owner).to_owned());
        let scope = if spec.owner == Some(ExporterOwner::LangfuseOtel) {
            self.config.langfuse_span_scope
        } else {
            SpanScope::Full
        };
        if owner.is_none() && scope == SpanScope::Full {
            return Ok(builder.with_span_processor(processor));
        }
        Ok(builder.with_span_processor(OverriddenBackendFilter {
            inner: Box::new(processor),
            owner,
            scope,
            sink: wire_sink_key(spec, self.factories),
            scope_source: Arc::clone(self.scope_source),
        }))
    }
}

pub fn build_tracer_provider(
    config: &Settings,
    factories: &ExporterFactories,
    scope_source: &Arc<dyn RequestScopeSource>,
    mut options: AssemblyOptions,
) -> ExportResult<SdkTracerProvider> {
    let baggage = options
        .baggage_processor
        .take()
        .unwrap_or_else(|| BaggageSpanProcessor::new(config.baggage_promoted_keys.clone(), None));
    let mut builder = SdkTracerProvider::builder()
        .with_resource(provider_resource(config))
        .with_span_processor(baggage);
    if let Some(sink) = options.exporter.take() {
        let processor = processor_for(sink, options.use_simple_processor);
        return Ok(builder.with_span_processor(processor).build());
    }
    let assembly = Assembly {
        config,
        factories,
        scope_source,
        options,
    };
    for spec in &config.exporters {
        if spec.requires_headers && spec.headers.is_none() {
            continue;
        }
        builder = assembly.add_exporter(builder, spec)?;
    }
    Ok(builder.build())
}

#[path = "tracer_provider_assembly/tests.rs"]
mod tests;
