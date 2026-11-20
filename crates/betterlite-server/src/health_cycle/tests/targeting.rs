#![cfg(test)]

use betterlite_router::deployment_identity::spec::DeploymentSpec;

use super::support::{
    FakeModels, FakeProvider, capture_logs, context, cycle, deployment, deployment_with_info,
    plain_request, policy, text_field,
};

fn shared_and_other() -> [DeploymentSpec; 2] {
    [
        deployment("shared", "shared-id", r#"{"model":"openai/shared"}"#),
        deployment("other", "other-id", r#"{"model":"openai/other"}"#),
    ]
}

#[tokio::test]
async fn a_model_id_wins_over_the_model_name() {
    let rows = shared_and_other();
    let mut request = plain_request(&rows);
    request.model = Some("other");
    request.model_id = Some("shared-id");
    let provider = FakeProvider::new();
    let result = cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert_eq!(provider.calls(), vec!["openai/shared".to_owned()]);
    assert_eq!(result.healthy_endpoints.len(), 1);
    assert_eq!(
        text_field(&result.healthy_endpoints[0], "model_id").as_deref(),
        Some("shared-id")
    );
}

#[tokio::test]
async fn an_unknown_model_id_keeps_the_whole_list() {
    let rows = shared_and_other();
    let mut request = plain_request(&rows);
    request.model_id = Some("ghost-id");
    let provider = FakeProvider::new();
    let result = cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert_eq!(
        provider.calls(),
        vec!["openai/shared".to_owned(), "openai/other".to_owned()]
    );
    assert_eq!(result.healthy_endpoints.len(), 2);
}

#[tokio::test]
async fn a_model_name_routes_only_its_own_deployments() {
    let (buffer, _guard) = capture_logs();
    let rows = shared_and_other();
    let mut request = plain_request(&rows);
    request.model = Some("shared");
    let context = context("cycle-target-1", "background");
    request.instrumentation = Some(&context);
    let provider = FakeProvider::new();
    cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert_eq!(provider.calls(), vec!["openai/shared".to_owned()]);
    let logs = buffer.text();
    assert!(logs.contains("requested_model_count=2"), "{logs}");
    assert!(logs.contains("post_model_filter_count=1"), "{logs}");
}

#[tokio::test]
async fn a_team_asks_for_its_own_public_copy_first() {
    let rows = [
        deployment_with_info(
            "team-internal",
            "team-copy-id",
            r#"{"model":"openai/team"}"#,
            r#"{"team_id":"team-1","team_public_model_name":"front"}"#,
        ),
        deployment("front", "public-id", r#"{"model":"openai/front"}"#),
    ];
    let mut request = plain_request(&rows);
    request.model = Some("front");
    request.team_id = Some("team-1");
    let provider = FakeProvider::new();
    cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert_eq!(provider.calls(), vec!["openai/team".to_owned()]);
}

#[tokio::test]
async fn a_public_name_without_a_team_routes_by_model_name() {
    let rows = [
        deployment_with_info(
            "team-internal",
            "team-copy-id",
            r#"{"model":"openai/team"}"#,
            r#"{"team_id":"team-1","team_public_model_name":"front"}"#,
        ),
        deployment("front", "public-id", r#"{"model":"openai/front"}"#),
    ];
    let mut request = plain_request(&rows);
    request.model = Some("front");
    let provider = FakeProvider::new();
    cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert_eq!(provider.calls(), vec!["openai/front".to_owned()]);
}

#[tokio::test]
async fn a_model_name_falls_back_to_the_deployed_model() {
    let rows = [deployment(
        "friendly-name",
        "leaf-id",
        r#"{"model":"vendor/actual"}"#,
    )];
    let mut request = plain_request(&rows);
    request.model = Some("vendor/actual");
    let provider = FakeProvider::new();
    let result = cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert_eq!(provider.calls(), vec!["vendor/actual".to_owned()]);
    assert_eq!(result.healthy_endpoints.len(), 1);
}

#[tokio::test]
async fn an_unmatched_model_name_probes_nothing() {
    let (buffer, _guard) = capture_logs();
    let rows = shared_and_other();
    let context = context("cycle-target", "background");
    let mut request = plain_request(&rows);
    request.model = Some("nowhere");
    request.instrumentation = Some(&context);
    let provider = FakeProvider::new();
    let result = cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert!(provider.calls().is_empty());
    assert!(result.healthy_endpoints.is_empty());
    assert!(result.unhealthy_endpoints.is_empty());
    let logs = buffer.text();
    assert!(logs.contains("reason=no_models_after_filter"), "{logs}");
    assert!(!logs.contains("health_check_cycle_start"), "{logs}");
}
