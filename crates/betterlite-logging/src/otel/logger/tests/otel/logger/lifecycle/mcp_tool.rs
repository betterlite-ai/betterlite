use betterlite_logging::otel::plumbing::routing::cache::MAX_CACHED_PROVIDERS;
use opentelemetry::trace::SpanId;

use crate::otel::support::{
    PreCall, Tenant, anchored, creds, event, failure, finished, harness, harness_for, inference,
    mcp_tool, mcp_tool_failure, mcp_tool_with_auth, names, owned_http, payload_start, phoenix_http,
    pre_call, root_trace_id, route_tenants, success, with_id,
};

#[tokio::test]
async fn mcp_tool_emits_one_whole_span_for_the_call() {
    let harness = harness(owned_http(), anchored());
    success(&harness.logger, event(with_id(mcp_tool(), "call-mcp"))).await;
    let spans = finished(&harness.spans);
    assert_eq!(spans.len(), 1);
    assert_eq!(
        names(&spans).first().map(String::as_str),
        Some("tools/call search")
    );
}

#[tokio::test]
async fn mcp_tool_emission_drops_the_stray_llm_carrier() {
    let harness = harness(owned_http(), anchored());
    pre_call(
        &harness.logger,
        PreCall {
            call_id: "call-mcp",
            model: "gpt-4o",
            no_upstream: false,
            dynamic: None,
            auth: None,
        },
    );
    success(&harness.logger, event(with_id(mcp_tool(), "call-mcp"))).await;
    success(&harness.logger, event(with_id(inference(), "call-mcp"))).await;
    let spans = finished(&harness.spans);
    assert_eq!(spans.len(), 2);
    assert_eq!(
        names(&spans).first().map(String::as_str),
        Some("tools/call search")
    );
    // the stray carrier is gone, so the later close emits deferred from the payload's own time
    assert_eq!(spans.get(1).expect("llm span").start_time, payload_start());
}

#[tokio::test]
async fn mcp_tool_dedups_the_success_and_failure_emissions() {
    let harness = harness(owned_http(), anchored());
    success(&harness.logger, event(with_id(mcp_tool(), "call-mcp"))).await;
    failure(
        &harness.logger,
        event(with_id(mcp_tool_failure(), "call-mcp")),
    )
    .await;
    assert_eq!(finished(&harness.spans).len(), 1);
}

#[tokio::test]
async fn mcp_tool_roots_a_detached_span_and_links_the_request_trace() {
    let harness = harness_for(phoenix_http(), anchored(), "arize_phoenix", None);
    success(
        &harness.logger,
        event(with_id(mcp_tool_with_auth(), "call-mcp")),
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
}

#[tokio::test]
async fn mcp_tool_emission_releases_the_project_routes_lease() {
    let harness = harness_for(phoenix_http(), anchored(), "arize_phoenix", None);
    success(
        &harness.logger,
        event(with_id(mcp_tool_with_auth(), "call-mcp")),
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

#[tokio::test]
async fn mcp_tool_dropping_the_stray_carrier_releases_its_route_lease() {
    let harness = harness(owned_http(), anchored());
    pre_call(
        &harness.logger,
        PreCall {
            call_id: "call-mcp",
            model: "gpt-4o",
            no_upstream: false,
            dynamic: Some(creds("S0")),
            auth: None,
        },
    );
    assert_eq!(harness.built.builds(), 1);
    success(&harness.logger, event(with_id(mcp_tool(), "call-mcp"))).await;
    route_tenants(
        &harness,
        "later-",
        1,
        MAX_CACHED_PROVIDERS,
        Tenant::Credentials(|index| format!("S{index}")),
    );
    // dropping the stray carrier released S0's lease, so the overflow eviction shuts its provider down
    assert!(harness.built.sink(0).is_shutdown_called());
}
