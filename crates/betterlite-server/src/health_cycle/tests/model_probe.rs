use std::time::Duration;

use betterlite_models::decisions::Json;

use super::endpoints::separate_results;
use super::ports::ProbeResponse;
use super::probe::{PreparedProbe, ProbeOutcome, execute_probe, prepare_probe};
use super::support::{
    FakeModels, FakeProvider, Reply, deployment, deployment_with_info, failure, number, policy,
    seeded_random, success, text,
};

const COMPLEXITY_ROUTER: &str = r#"{"model":"auto_router/complexity_router","complexity_router_config":{"tiers":{"SIMPLE":"leaf"}}}"#;

#[tokio::test]
async fn a_strategy_router_is_answered_without_a_probe() {
    let router = deployment("outer", "outer-id", COMPLEXITY_ROUTER);
    let models = FakeModels::default();
    let policy = policy();
    let mut random = seeded_random();
    let prepared = prepare_probe(&router, &models, &policy, &mut random);
    assert_eq!(prepared, PreparedProbe::Skipped);
    let provider = FakeProvider::new();
    let outcome = execute_probe(&provider, &prepared).await;
    assert_eq!(outcome, ProbeOutcome::Healthy(ProbeResponse::default()));
    assert!(provider.calls().is_empty());
}

#[tokio::test]
async fn the_probe_uses_the_resolved_mode_and_the_policy_prompt() {
    let leaf = deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#);
    let mut models = FakeModels::default();
    models.modes.insert("openai/leaf".to_owned(), "chat");
    let policy = policy();
    let mut random = seeded_random();
    let PreparedProbe::Ready { request, timeout } =
        prepare_probe(&leaf, &models, &policy, &mut random)
    else {
        panic!("a leaf deployment is probed");
    };
    assert_eq!(request.mode.as_deref(), Some("chat"));
    assert_eq!(request.prompt, "test from litellm");
    assert_eq!(request.input, vec!["test from litellm".to_owned()]);
    assert_eq!(timeout, Duration::from_secs(60));
    assert_eq!(request.params.get("max_tokens"), Some(&number(16)));
    let Some(Json::List(messages)) = request.params.get("messages") else {
        panic!("the probe carries the random message");
    };
    assert_eq!(messages.len(), 1);
    let Some(Json::Object(message)) = messages.first() else {
        panic!("the message is an object");
    };
    assert_eq!(message.get("role"), Some(&text("user")));
    let drawn = message.get("content").expect("the message has content");
    assert!([text("Hey how's it going?"), text("What's 1 + 1?")].contains(drawn));
    let provider = FakeProvider::new();
    let prepared = PreparedProbe::Ready {
        request: request.clone(),
        timeout,
    };
    let outcome = execute_probe(&provider, &prepared).await;
    assert_eq!(outcome, ProbeOutcome::Healthy(success("openai/leaf")));
    assert_eq!(provider.requests(), vec![request]);
}

#[test]
fn an_explicit_model_info_mode_wins_over_the_catalog() {
    let leaf = deployment_with_info(
        "leaf",
        "leaf-id",
        r#"{"model":"openai/leaf"}"#,
        r#"{"mode":"embedding"}"#,
    );
    let mut models = FakeModels::default();
    models.modes.insert("openai/leaf".to_owned(), "chat");
    let policy = policy();
    let mut random = seeded_random();
    let PreparedProbe::Ready { request, .. } = prepare_probe(&leaf, &models, &policy, &mut random)
    else {
        panic!("a leaf deployment is probed");
    };
    assert_eq!(request.mode.as_deref(), Some("embedding"));
}

#[test]
fn a_deployment_timeout_overrides_the_policy_default() {
    let leaf = deployment_with_info(
        "leaf",
        "leaf-id",
        r#"{"model":"openai/leaf"}"#,
        r#"{"health_check_timeout":5}"#,
    );
    let models = FakeModels::default();
    let policy = policy();
    let mut random = seeded_random();
    let PreparedProbe::Ready { timeout, .. } = prepare_probe(&leaf, &models, &policy, &mut random)
    else {
        panic!("a leaf deployment is probed");
    };
    assert_eq!(timeout, Duration::from_secs(5));
}

#[tokio::test]
async fn a_provider_failure_becomes_an_unhealthy_row() {
    let leaf = deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#);
    let models = FakeModels::default();
    let policy = policy();
    let mut random = seeded_random();
    let prepared = prepare_probe(&leaf, &models, &policy, &mut random);
    let provider = FakeProvider::new().with_reply(
        "openai/leaf",
        Reply::Failure {
            status: 429,
            message: "rate limited",
        },
    );
    let outcome = execute_probe(&provider, &prepared).await;
    let result = separate_results(&[leaf], &[outcome], true);
    assert!(result.healthy_endpoints.is_empty());
    let endpoint = result
        .unhealthy_endpoints
        .first()
        .expect("the failed probe is reported");
    assert_eq!(endpoint.get("error"), Some(&text("rate limited")));
    assert_eq!(endpoint.get("exception_status"), Some(&number(429)));
    assert_eq!(endpoint.get("model_id"), Some(&text("leaf-id")));
    assert_eq!(
        result.exceptions_by_model_id.get("leaf-id"),
        Some(&failure(429, "rate limited"))
    );
}

#[test]
fn a_settings_error_fails_only_its_own_probe() {
    let leaf = deployment_with_info(
        "leaf",
        "leaf-id",
        r#"{"model":"openai/leaf"}"#,
        r#"{"health_check_max_tokens":"plenty"}"#,
    );
    let models = FakeModels::default();
    let policy = policy();
    let mut random = seeded_random();
    let prepared = prepare_probe(&leaf, &models, &policy, &mut random);
    let PreparedProbe::Failed(failure) = prepared else {
        panic!("a malformed token limit fails the probe");
    };
    assert_eq!(failure.status_code, 500);
    assert!(failure.message.contains("health_check_max_tokens"));
}
