#![cfg(test)]

use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_router::model_catalog_lookup::alias::{AliasMap, AliasTarget};

use super::cycle::{HealthCycleDeps, perform_health_check};
use super::endpoints::HealthCheckResult;
use super::error::HealthCycleError;
use super::support::{
    FakeModels, FakeProvider, Fixture, Reply, capture_logs, context, cycle, deployment,
    deployment_with_info, plain_request, policy, seeded_random, text, text_field,
};

const COMPLEXITY_ROUTER: &str = r#"{"model":"auto_router/complexity_router","complexity_router_config":{"tiers":{"SIMPLE":"leaf"}}}"#;

// B4444 runs the background loop on the server's TaskTracker, so the whole cycle future has to be Send
#[test]
fn the_cycle_future_is_send() {
    let rows = [deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#)];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    let mut request = plain_request(&rows);
    request.lookup = Some(&lookup);
    let provider = FakeProvider::new();
    let models = FakeModels::default();
    let probe_policy = policy();
    let mut random = seeded_random();
    let deps = HealthCycleDeps {
        provider: &provider,
        models: &models,
        policy: &probe_policy,
        random: &mut random,
    };
    let _sendable: Box<
        dyn Future<Output = Result<HealthCheckResult, HealthCycleError>> + Send + '_,
    > = Box::new(perform_health_check(request, deps));
}

#[tokio::test]
async fn dependency_probes_leave_the_answer() {
    let (buffer, _guard) = capture_logs();
    let rows = [
        deployment("outer", "outer-id", COMPLEXITY_ROUTER),
        deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#),
    ];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    let context = context("cycle-deps", "background");
    let mut request = plain_request(&rows);
    request.model_id = Some("outer-id");
    request.lookup = Some(&lookup);
    request.instrumentation = Some(&context);
    let provider = FakeProvider::new();
    let result = cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert_eq!(provider.calls(), vec!["openai/leaf".to_owned()]);
    assert!(result.unhealthy_endpoints.is_empty());
    assert_eq!(result.healthy_endpoints.len(), 1);
    let endpoint = result
        .healthy_endpoints
        .first()
        .expect("the router is reported healthy");
    assert_eq!(endpoint.get("model_id"), Some(&text("outer-id")));
    assert_eq!(
        endpoint.get("model"),
        Some(&text("auto_router/complexity_router"))
    );
    let logs = buffer.text();
    assert!(logs.contains("model_count=2"), "{logs}");
    assert!(logs.contains("peak_in_flight=2"), "{logs}");
}

#[tokio::test]
async fn a_red_dependency_moves_its_router_to_unhealthy() {
    let rows = [
        deployment("outer", "outer-id", COMPLEXITY_ROUTER),
        deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#),
    ];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    let mut request = plain_request(&rows);
    request.model_id = Some("outer-id");
    request.lookup = Some(&lookup);
    let provider = FakeProvider::new().with_reply(
        "openai/leaf",
        Reply::Failure {
            status: 500,
            message: "leaf is down",
        },
    );
    let result = cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert!(result.healthy_endpoints.is_empty());
    assert_eq!(result.unhealthy_endpoints.len(), 1);
    let endpoint = result
        .unhealthy_endpoints
        .first()
        .expect("the router is reported unhealthy");
    assert_eq!(endpoint.get("model_id"), Some(&text("outer-id")));
    let error = text_field(endpoint, "error").expect("the moved router carries a verdict");
    assert!(error.contains("tier model 'leaf'"), "{error}");
}

#[tokio::test]
async fn a_lookup_failure_leaves_the_cycle_with_an_error() {
    let rows = [deployment(
        "outer",
        "outer-id",
        r#"{"model":"auto_router/complexity_router","complexity_router_config":{"tiers":{"SIMPLE":"broken"}}}"#,
    )];
    let mut aliases = AliasMap::new();
    aliases.insert(
        "broken".to_owned(),
        AliasTarget::Item {
            model: None,
            hidden: Some(Json::Bool(false)),
        },
    );
    let fixture = Fixture::with_alias_map(&rows, aliases);
    let lookup = fixture.lookup();
    let mut request = plain_request(&rows);
    request.model_id = Some("outer-id");
    request.lookup = Some(&lookup);
    let provider = FakeProvider::new();
    let error = cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect_err("the malformed alias fails the lookup");
    assert!(matches!(error, HealthCycleError::Router(_)));
    assert!(provider.calls().is_empty());
}

#[tokio::test]
async fn the_details_flag_reaches_the_rows() {
    let rows = [deployment_with_info(
        "leaf",
        "leaf-id",
        r#"{"model":"openai/leaf","api_base":"https://leaf.example"}"#,
        "{}",
    )];
    let mut request = plain_request(&rows);
    request.details = false;
    let provider = FakeProvider::new().with_reply(
        "openai/leaf",
        Reply::Failure {
            status: 500,
            message: "boom",
        },
    );
    let result = cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    let endpoint = result
        .unhealthy_endpoints
        .first()
        .expect("the failed probe is reported");
    assert_eq!(endpoint.get("error"), None);
    assert_eq!(endpoint.get("api_base"), None);
}

#[tokio::test]
async fn the_cycle_logs_its_start_and_completion() {
    let (buffer, _guard) = capture_logs();
    let rows = [
        deployment("a", "a-id", r#"{"model":"openai/a"}"#),
        deployment("b", "b-id", r#"{"model":"openai/b"}"#),
    ];
    let mut request = plain_request(&rows);
    let context = context("cycle-5", "background");
    request.instrumentation = Some(&context);
    let provider = FakeProvider::new().with_reply(
        "openai/b",
        Reply::Failure {
            status: 500,
            message: "boom",
        },
    );
    let result = cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert_eq!(result.healthy_endpoints.len(), 1);
    assert_eq!(result.unhealthy_endpoints.len(), 1);
    let logs = buffer.text();
    assert!(logs.contains("health_check_cycle_start"), "{logs}");
    assert!(logs.contains("requested_model_count=2"), "{logs}");
    assert!(logs.contains("post_model_filter_count=2"), "{logs}");
    assert!(logs.contains("deduped_model_count=2"), "{logs}");
    assert!(logs.contains("max_concurrency=None"), "{logs}");
    assert!(logs.contains("health_check_cycle_complete"), "{logs}");
    assert!(logs.contains("model_count=2"), "{logs}");
    assert!(logs.contains("healthy_count=1"), "{logs}");
    assert!(logs.contains("unhealthy_count=1"), "{logs}");
    assert!(logs.contains("duration_ms="), "{logs}");
    assert!(logs.contains("thread_count="), "{logs}");
    assert!(logs.contains("rss_mb="), "{logs}");
}
