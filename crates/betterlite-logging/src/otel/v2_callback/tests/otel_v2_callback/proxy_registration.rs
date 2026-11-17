#![cfg(test)]

use std::sync::Arc;

use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor};

use betterlite_logging::otel::v2_callback::callback::V2Callback;
use betterlite_logging::otel::v2_callback::registry::{CallbackEntry, CallbackList, V2Registry};

use super::support::{construction_parts, settings};

fn provider(spans: &InMemorySpanExporter) -> SdkTracerProvider {
    SdkTracerProvider::builder()
        .with_span_processor(SimpleSpanProcessor::new(spans.clone()))
        .build()
}

#[test]
fn one_entry_per_list_and_the_first_callback_keeps_the_slot() {
    let spans = InMemorySpanExporter::default();
    let tracer_provider = provider(&spans);
    let registry = V2Registry::default();
    let first = V2Callback::new(
        construction_parts(settings(), &spans, Some(tracer_provider.clone())),
        &registry,
    )
    .expect("first callback");
    let _second = V2Callback::new(
        construction_parts(settings(), &spans, Some(tracer_provider)),
        &registry,
    )
    .expect("second callback");
    for list in CallbackList::ALL {
        let entries = registry.entries(list);
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0], CallbackEntry::OpenTelemetry(_)));
    }
    let slot = registry.registered_v2_logger().expect("slot");
    assert!(Arc::ptr_eq(&slot, &first));
}

#[test]
fn an_otel_module_entry_blocks_the_lists_but_not_the_slot() {
    let spans = InMemorySpanExporter::default();
    let registry = V2Registry::default();
    registry.add_named_callback("litellm.integrations.opentelemetry.other");
    let callback = V2Callback::new(
        construction_parts(settings(), &spans, Some(provider(&spans))),
        &registry,
    )
    .expect("callback");
    for list in CallbackList::ALL {
        let entries = registry.entries(list);
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0], CallbackEntry::Named(_)));
    }
    let slot = registry.registered_v2_logger().expect("slot");
    assert!(Arc::ptr_eq(&slot, &callback));
}

#[test]
fn a_foreign_callback_keeps_the_otel_registration() {
    let spans = InMemorySpanExporter::default();
    let registry = V2Registry::default();
    registry.add_named_callback("my.extensions.callback");
    let _callback = V2Callback::new(
        construction_parts(settings(), &spans, Some(provider(&spans))),
        &registry,
    )
    .expect("callback");
    for list in CallbackList::ALL {
        let entries = registry.entries(list);
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0], CallbackEntry::Named(_)));
        assert!(matches!(entries[1], CallbackEntry::OpenTelemetry(_)));
    }
    assert!(registry.registered_v2_logger().is_some());
}
