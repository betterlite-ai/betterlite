use crate::otel::config::ExporterSpec;
use crate::otel::config::destination::OtelDestination;
use crate::otel::plumbing::exporters::{ExporterFactories, Transport};
use crate::otel::plumbing::processors::processor_for;
use opentelemetry::{Key, KeyValue, Value};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SpanProcessor;
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::Arc;
pub trait DestinationProcessorFactory: Send + Sync + Debug {
    fn build(
        &self,
        destination: &OtelDestination,
        resource: &Resource,
    ) -> Option<Arc<dyn SpanProcessor>>;
}

pub struct ExporterDestinationFactory {
    pub factories: Arc<ExporterFactories>,
}

impl Debug for ExporterDestinationFactory {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str("ExporterDestinationFactory")
    }
}

impl DestinationProcessorFactory for ExporterDestinationFactory {
    fn build(
        &self,
        destination: &OtelDestination,
        resource: &Resource,
    ) -> Option<Arc<dyn SpanProcessor>> {
        let fields = destination.as_ref();
        let kind = fields.protocol.as_deref().unwrap_or("otlp_http");
        if self.factories.exporter_transport(kind) == Transport::Headerless {
            tracing::debug!(
                endpoint = %fields.endpoint,
                protocol = kind,
                "otel: tenant destination has no OTLP transport"
            );
            return None;
        }
        let spec = ExporterSpec {
            kind: kind.to_owned(),
            endpoint: Some(fields.endpoint.clone()),
            traces_endpoint: None,
            headers: Some(destination.header_string()),
            owner: None,
            options: None,
            use_simple_processor: Some(false),
            requires_headers: false,
        };
        let sink = self.factories.exporter_from_spec(&spec).map_err(|error| {
            tracing::debug!(endpoint = %fields.endpoint, %error, "otel: tenant destination did not build");
        }).ok()?;
        let mut processor = processor_for(sink, Some(false));
        processor.set_resource(&destination_resource(resource, destination));
        Some(Arc::new(processor))
    }
}

#[must_use]
pub fn destination_resource(base: &Resource, destination: &OtelDestination) -> Resource {
    let extra = destination
        .as_ref()
        .resource_attributes
        .iter()
        .map(|(key, value)| KeyValue::new(Key::new(key.clone()), Value::from(value.clone())));
    let merged: Vec<KeyValue> = base
        .iter()
        .map(|(key, value)| KeyValue::new(key.clone(), value.clone()))
        .chain(extra)
        .collect();
    let builder = Resource::builder_empty();
    match base.schema_url() {
        Some(url) => builder.with_schema_url(merged, url.to_owned()).build(),
        None => builder.with_attributes(merged).build(),
    }
}

#[path = "destination/tests.rs"]
mod tests;
