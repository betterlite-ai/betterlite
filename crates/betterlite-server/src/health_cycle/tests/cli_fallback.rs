use betterlite_router::deployment_identity::spec::DeploymentSpec;

use super::support::{
    FakeModels, FakeProvider, capture_logs, context, cycle, deployment, plain_request, policy,
};

#[tokio::test]
async fn a_cli_model_builds_a_synthetic_row_that_the_dedupe_drops() {
    let (buffer, _guard) = capture_logs();
    let rows: Vec<DeploymentSpec> = Vec::new();
    let context = context("cycle-cli", "cli");
    let mut request = plain_request(&rows);
    request.cli_model = Some("gpt-4o");
    request.instrumentation = Some(&context);
    let provider = FakeProvider::new();
    let result = cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert!(result.healthy_endpoints.is_empty());
    assert!(result.unhealthy_endpoints.is_empty());
    assert!(result.exceptions_by_model_id.is_empty());
    assert!(provider.calls().is_empty());
    let logs = buffer.text();
    assert!(logs.contains("health_check_cycle_start"), "{logs}");
    assert!(logs.contains("requested_model_count=1"), "{logs}");
    assert!(logs.contains("deduped_model_count=0"), "{logs}");
    assert!(!logs.contains("reason=no_models"), "{logs}");
}

#[tokio::test]
async fn an_empty_list_without_a_cli_model_skips_the_cycle() {
    let (buffer, _guard) = capture_logs();
    let rows: Vec<DeploymentSpec> = Vec::new();
    let context = context("cycle-empty", "background");
    let mut request = plain_request(&rows);
    request.instrumentation = Some(&context);
    let provider = FakeProvider::new();
    let result = cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert!(result.healthy_endpoints.is_empty());
    assert!(provider.calls().is_empty());
    let logs = buffer.text();
    assert!(logs.contains("health_check_cycle_skipped"), "{logs}");
    assert!(logs.contains("reason=no_models"), "{logs}");
    assert!(!logs.contains("health_check_cycle_start"), "{logs}");
}

#[tokio::test]
async fn a_cli_model_is_ignored_when_the_list_is_not_empty() {
    let (buffer, _guard) = capture_logs();
    let rows = [
        deployment("a", "a-id", r#"{"model":"openai/a"}"#),
        deployment("b", "b-id", r#"{"model":"openai/b"}"#),
    ];
    let context = context("cycle-cli", "cli");
    let mut request = plain_request(&rows);
    request.cli_model = Some("gpt-4o");
    request.instrumentation = Some(&context);
    let provider = FakeProvider::new();
    cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert_eq!(
        provider.calls(),
        vec!["openai/a".to_owned(), "openai/b".to_owned()]
    );
    let logs = buffer.text();
    assert!(logs.contains("requested_model_count=2"), "{logs}");
    assert!(logs.contains("deduped_model_count=2"), "{logs}");
}
