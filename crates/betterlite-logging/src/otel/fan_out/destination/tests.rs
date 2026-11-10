#![cfg(test)]

use std::collections::BTreeMap;

use opentelemetry::{Key, KeyValue, Value};
use opentelemetry_sdk::Resource;
use rstest::rstest;

use super::{DestinationProcessorFactory as _, ExporterDestinationFactory, destination_resource};
use crate::otel::config::destination::{DestinationFields, OtelDestination, SpanScope};
use crate::otel::fan_out::tests::support::factories;

fn destination(endpoint: &str, protocol: Option<&str>) -> OtelDestination {
    OtelDestination::from(DestinationFields {
        endpoint: endpoint.to_owned(),
        headers: BTreeMap::new(),
        resource_attributes: BTreeMap::new(),
        callback_name: None,
        protocol: protocol.map(str::to_owned),
        span_scope: SpanScope::Full,
    })
}

fn tenant_resource() -> Resource {
    Resource::builder_empty()
        .with_attribute(KeyValue::new("service.name", "betterlite"))
        .with_attribute(KeyValue::new("deployment.environment", "test"))
        .build()
}

#[test]
fn an_otlp_http_destination_builds_a_batch_processor() {
    let factory = ExporterDestinationFactory {
        factories: factories(),
    };
    let processor = factory
        .build(
            &destination("http://collector:4318", None),
            &tenant_resource(),
        )
        .expect("an otlp destination builds");
    processor.shutdown().expect("the processor closes");
}

#[rstest]
#[case::console("console")]
#[case::empty("")]
#[case::in_memory("in_memory")]
#[case::memory("memory")]
#[case::unregistered("jaeger")]
fn a_headerless_transport_is_not_buildable(#[case] protocol: &str) {
    let factory = ExporterDestinationFactory {
        factories: factories(),
    };
    assert!(
        factory
            .build(
                &destination("http://collector:4318", Some(protocol)),
                &tenant_resource()
            )
            .is_none()
    );
}

#[test]
fn destination_resource_lets_the_tenant_attributes_win() {
    let mut fields = DestinationFields {
        endpoint: "http://collector:4318".to_owned(),
        headers: BTreeMap::new(),
        resource_attributes: BTreeMap::new(),
        callback_name: None,
        protocol: None,
        span_scope: SpanScope::Full,
    };
    fields
        .resource_attributes
        .insert("service.name".to_owned(), "tenant-service".to_owned());
    fields
        .resource_attributes
        .insert("tenant.id".to_owned(), "t-1".to_owned());
    let resource = destination_resource(&tenant_resource(), &OtelDestination::from(fields));
    assert_eq!(
        resource.get(&Key::new("service.name")),
        Some(Value::from("tenant-service"))
    );
    assert_eq!(
        resource.get(&Key::new("tenant.id")),
        Some(Value::from("t-1"))
    );
    assert_eq!(
        resource.get(&Key::new("deployment.environment")),
        Some(Value::from("test"))
    );
}

#[test]
fn destination_resource_keeps_the_base_resource_when_the_destination_adds_none() {
    let base = Resource::builder_empty()
        .with_schema_url(
            [KeyValue::new("service.name", "betterlite")],
            "http://schema.example/1",
        )
        .build();
    let resource = destination_resource(&base, &destination("http://collector:4318", None));
    assert_eq!(resource.schema_url(), Some("http://schema.example/1"));
    assert_eq!(
        resource.get(&Key::new("service.name")),
        Some(Value::from("betterlite"))
    );
}
