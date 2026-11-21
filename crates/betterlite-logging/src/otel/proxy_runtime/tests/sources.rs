#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use opentelemetry::Context;
use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::InMemorySpanExporter;
use rstest::rstest;

use crate::langfuse_observations::attributes::keys::TRACE_NAME;
use crate::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use crate::otel::langfuse_root::{RootSpanSink, RootSpanSource};
use crate::otel::logger::v2::RequestTraceSource;
use crate::otel::logger_runtime::tests::support::{destination, provider_exporting};
use crate::otel::plumbing::context::context_slots::AnchoredSpan;
use crate::otel::plumbing::context::destination_routing::TENANT_DESTINATION_MODE_ENV;
use crate::otel::proxy_runtime::slot::{anchor_request_root, request_slot_scope, set_destinations};
use crate::otel::proxy_runtime::sources::ProxyTraceSources;

use super::support::{anchored, invalid_anchor};

#[tokio::test]
async fn the_scope_carries_the_slot_destinations() {
    let sources = ProxyTraceSources::new(&BTreeMap::<String, String>::new());
    assert!(sources.scope().destinations.is_empty());
    let carried = vec![destination("langfuse_otel")];
    request_slot_scope(async {
        set_destinations(carried.clone());
        assert_eq!(
            sources.scope(),
            RequestOtelScope {
                destinations: carried.clone(),
                additive: false,
            }
        );
    })
    .await;
    assert!(sources.scope().destinations.is_empty());
}

#[rstest]
#[case::additive(Some("additive"), true)]
#[case::mixed_case(Some("  AdDiTiVe  "), true)]
#[case::override_mode(Some("override"), false)]
#[case::empty(Some(""), false)]
#[case::unset(None, false)]
fn the_additive_flag_follows_the_tenant_destination_mode(
    #[case] value: Option<&str>,
    #[case] additive: bool,
) {
    let mut lookup = BTreeMap::new();
    if let Some(value) = value {
        lookup.insert(TENANT_DESTINATION_MODE_ENV.to_owned(), value.to_owned());
    }
    assert_eq!(ProxyTraceSources::new(&lookup).scope().additive, additive);
}

#[tokio::test]
async fn the_root_sink_writes_onto_the_anchored_live_span() {
    let exporter = InMemorySpanExporter::default();
    let provider = provider_exporting(&exporter);
    let sources = ProxyTraceSources::new(&BTreeMap::<String, String>::new());
    let (slot, context) = request_slot_scope(async {
        let span = provider.tracer("proxy_runtime_test").start("SERVER");
        let root = AnchoredSpan {
            context: span.span_context().clone(),
            http_route: None,
        };
        anchor_request_root(root.clone(), Context::current_with_span(span));
        let sink: Arc<dyn RootSpanSink> = sources
            .request_root_span()
            .expect("the slot carries the live root");
        assert!(sink.is_recording());
        sink.set_attribute(TRACE_NAME, "kept".to_owned());
        root.context
    })
    .await;
    assert!(sources.trace_context().root.is_none());
    drop(slot);
    let finished = exporter
        .get_finished_spans()
        .expect("the root span was exported");
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].span_context, context);
    assert!(finished[0].attributes.iter().any(|attribute| {
        attribute.key.as_str() == TRACE_NAME && attribute.value == Value::from("kept")
    }));
}

#[tokio::test]
async fn an_invalid_anchor_keeps_the_earlier_root() {
    let exporter = InMemorySpanExporter::default();
    let provider = provider_exporting(&exporter);
    let sources = ProxyTraceSources::new(&BTreeMap::<String, String>::new());
    request_slot_scope(async {
        anchor_request_root(invalid_anchor(), Context::new());
        assert!(sources.trace_context().root.is_none());
        let span = provider.tracer("proxy_runtime_test").start("SERVER");
        let root = AnchoredSpan {
            context: span.span_context().clone(),
            http_route: None,
        };
        anchor_request_root(root.clone(), Context::current_with_span(span));
        anchor_request_root(invalid_anchor(), Context::new());
        assert_eq!(sources.trace_context().root, Some(root));
        let sink: Arc<dyn RootSpanSink> = sources
            .request_root_span()
            .expect("the earlier live root stays");
        assert!(sink.is_recording());
        sink.set_attribute(TRACE_NAME, "kept".to_owned());
    })
    .await;
    let finished = exporter
        .get_finished_spans()
        .expect("the root span was exported");
    assert_eq!(finished.len(), 1);
    assert!(
        finished[0]
            .attributes
            .iter()
            .any(|attribute| attribute.key.as_str() == TRACE_NAME)
    );
}
