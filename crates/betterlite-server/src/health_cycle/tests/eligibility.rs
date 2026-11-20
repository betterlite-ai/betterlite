#![cfg(test)]

use betterlite_router::deployment_identity::spec::DeploymentSpec;

use super::support::{
    FakeModels, FakeProvider, Fixture, cycle, deployment, deployment_with_info, plain_request,
    policy,
};

fn enabled_and_disabled() -> Vec<DeploymentSpec> {
    vec![
        deployment_with_info("leaf", "enabled-id", r#"{"model":"openai/enabled"}"#, "{}"),
        deployment_with_info(
            "leaf",
            "disabled-id",
            r#"{"model":"openai/disabled"}"#,
            r#"{"disable_background_health_check":true}"#,
        ),
    ]
}

#[tokio::test]
async fn skipping_disabled_models_drops_the_opted_out_rows() {
    let rows = enabled_and_disabled();
    let mut request = plain_request(&rows);
    request.skip_disabled = true;
    let provider = FakeProvider::new();
    cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert_eq!(provider.calls(), vec!["openai/enabled".to_owned()]);
}

#[tokio::test]
async fn disabling_the_skip_probes_every_row() {
    let rows = enabled_and_disabled();
    let mut request = plain_request(&rows);
    request.skip_disabled = false;
    let provider = FakeProvider::new();
    cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert_eq!(
        provider.calls(),
        vec!["openai/enabled".to_owned(), "openai/disabled".to_owned()]
    );
}

const COMPLEXITY_ROUTER: &str = r#"{"model":"auto_router/complexity_router","complexity_router_config":{"tiers":{"SIMPLE":"leaf"}}}"#;

#[tokio::test]
async fn an_opted_out_dependency_cannot_re_enter_through_its_router() {
    let rows = [
        deployment("outer", "outer-id", COMPLEXITY_ROUTER),
        deployment_with_info(
            "leaf",
            "leaf-id",
            r#"{"model":"openai/leaf"}"#,
            r#"{"disable_background_health_check":true}"#,
        ),
    ];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    let mut request = plain_request(&rows);
    request.model_id = Some("outer-id");
    request.lookup = Some(&lookup);
    request.skip_disabled = true;
    let provider = FakeProvider::new();
    let result = cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert!(provider.calls().is_empty());
    assert_eq!(result.healthy_endpoints.len(), 1);
}

#[tokio::test]
async fn without_the_skip_a_dependency_is_probed() {
    let rows = [
        deployment("outer", "outer-id", COMPLEXITY_ROUTER),
        deployment_with_info(
            "leaf",
            "leaf-id",
            r#"{"model":"openai/leaf"}"#,
            r#"{"disable_background_health_check":true}"#,
        ),
    ];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    let mut request = plain_request(&rows);
    request.model_id = Some("outer-id");
    request.lookup = Some(&lookup);
    let provider = FakeProvider::new();
    cycle(request, &provider, &FakeModels::default(), &policy())
        .await
        .expect("the cycle runs");
    assert_eq!(provider.calls(), vec!["openai/leaf".to_owned()]);
}
