use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::Arc;

use opentelemetry::{Key, KeyValue, Value};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SpanProcessor;

use crate::otel::config::ExporterSpec;
use crate::otel::config::destination::OtelDestination;
use crate::otel::plumbing::exporters::{ExporterFactories, Transport};
use crate::otel::plumbing::processors::processor_for;

// The fan-out's injected builder for one destination's export pipeline; nothing here reads a registry.
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
    // litellm.integrations.otel.plumbing.providers._destination_processor
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
        let sink = match self.factories.exporter_from_spec(&spec) {
            Ok(sink) => sink,
            Err(error) => {
                tracing::debug!(
                    endpoint = %fields.endpoint,
                    %error,
                    "otel: tenant destination exporter did not build"
                );
                return None;
            }
        };
        let mut processor = processor_for(sink, Some(false));
        processor.set_resource(resource);
        Some(Arc::new(processor))
    }
}

// litellm.integrations.otel.plumbing.providers._for_destination's resource merge, carried by the destination processor
#[must_use]
pub fn destination_resource(base: &Resource, destination: &OtelDestination) -> Resource {
    let mut merged: Vec<KeyValue> = base
        .iter()
        .map(|(key, value)| KeyValue::new(key.clone(), value.clone()))
        .collect();
    for (key, value) in &destination.as_ref().resource_attributes {
        merged.push(KeyValue::new(
            Key::new(key.clone()),
            Value::from(value.clone()),
        ));
    }
    let builder = Resource::builder_empty();
    match base.schema_url() {
        Some(url) => builder.with_schema_url(merged, url.to_owned()).build(),
        None => builder.with_attributes(merged).build(),
    }
}

#[path = "destination/tests.rs"]
mod tests;
