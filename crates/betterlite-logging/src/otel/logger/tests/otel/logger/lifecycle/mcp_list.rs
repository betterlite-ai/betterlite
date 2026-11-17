use betterlite_logging::otel::plumbing::routing::cache::MAX_CACHED_PROVIDERS;
use opentelemetry::trace::SpanId;

use crate::otel::support::{
    Tenant, anchored, event, failure, finished, harness, harness_for, mcp_list, mcp_list_failure,
    mcp_list_with_auth, names, owned_http, phoenix_http, root_trace_id, route_tenants, success,
    with_id,
};

#[tokio::test]
async fn mcp_list_emits_one_whole_span_for_the_call() {
    let harness = harness(owned_http(), anchored());
    success(&harness.logger, event(with_id(mcp_list(), "call-list"))).await;
    let spans = finished(&harness.spans);
    assert_eq!(spans.len(), 1);
    assert_eq!(
        names(&spans).first().map(String::as_str),
        Some("tools/list")
    );
}

#[tokio::test]
async fn mcp_list_dedups_the_success_and_failure_emissions() {
    let harness = harness(owned_http(), anchored());
    success(&harness.logger, event(with_id(mcp_list(), "call-list"))).await;
    failure(
        &harness.logger,
        event(with_id(mcp_list_failure(), "call-list")),
    )
    .await;
    assert_eq!(finished(&harness.spans).len(), 1);
}

#[tokio::test]
async fn mcp_list_roots_a_detached_span_and_links_the_request_trace() {
    let harness = harness_for(phoenix_http(), anchored(), "arize_phoenix", None);
    success(
        &harness.logger,
        event(with_id(mcp_list_with_auth(), "call-list")),
    )
    .await;
    let spans = finished(&harness.built.sink(0));
    assert_eq!(spans.len(), 1);
    let span = spans.first().expect("span");
    assert_eq!(span.parent_span_id, SpanId::INVALID);
    assert!(
        span.links
            .iter()
            .any(|link| link.span_context.trace_id() == root_trace_id())
    );
    let specs = harness.built.specs();
    let headers = specs
        .first()
        .and_then(|spec| spec.headers.clone())
        .unwrap_or_default();
    assert!(headers.contains("x-project-name=proj"));
}

#[tokio::test]
async fn mcp_list_emission_releases_the_project_routes_lease() {
    let harness = harness_for(phoenix_http(), anchored(), "arize_phoenix", None);
    success(
        &harness.logger,
        event(with_id(mcp_list_with_auth(), "call-list")),
    )
    .await;
    assert_eq!(harness.built.builds(), 1);
    route_tenants(
        &harness,
        "project-",
        1,
        MAX_CACHED_PROVIDERS,
        Tenant::Project(|index| format!("proj{index}")),
    );
    // the emission released the project route's lease, so the overflow eviction shuts it down at once
    assert!(harness.built.sink(0).is_shutdown_called());
}
