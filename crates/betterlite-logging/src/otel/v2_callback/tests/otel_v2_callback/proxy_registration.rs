#![cfg(test)]

use std::sync::Arc;

use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor};

use betterlite_logging::otel::v2_callback::callback::V2Callback;
use betterlite_logging::otel::v2_callback::registry::{CALLBACK_LISTS, CallbackEntry, V2Registry};
use betterlite_logging::registry::phases::CallbackList;

use super::support::{construction_parts, settings};

fn provider(spans: &InMemorySpanExporter) -> SdkTracerProvider {
    SdkTracerProvider::builder()
        .with_span_processor(SimpleSpanProcessor::new(spans.clone()))
        .build()
}

fn list_entries(registry: &V2Registry) -> Vec<Vec<CallbackEntry>> {
    CALLBACK_LISTS
        .iter()
        .map(|list| registry.entries(*list))
        .collect()
}

#[test]
fn the_four_proxy_lists_are_pinned() {
    assert_eq!(
        CALLBACK_LISTS,
        [
            CallbackList::Service,
            CallbackList::Input,
            CallbackList::Success,
            CallbackList::Failure,
        ]
    );
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
    let entries = list_entries(&registry);
    assert_eq!(entries.len(), CALLBACK_LISTS.len());
    assert!(
        entries
            .iter()
            .all(|list| list.len() == 1 && matches!(list[0], CallbackEntry::OpenTelemetry(_)))
    );
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
    let entries = list_entries(&registry);
    assert_eq!(entries.len(), CALLBACK_LISTS.len());
    assert!(
        entries
            .iter()
            .all(|list| list.len() == 1 && matches!(list[0], CallbackEntry::Named(_)))
    );
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
    let entries = list_entries(&registry);
    assert_eq!(entries.len(), CALLBACK_LISTS.len());
    assert!(entries.iter().all(|list| list.len() == 2
        && matches!(list[0], CallbackEntry::Named(_))
        && matches!(list[1], CallbackEntry::OpenTelemetry(_))));
    assert!(registry.registered_v2_logger().is_some());
}
