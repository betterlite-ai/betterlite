#![cfg(test)]

use std::time::Duration;

use betterlite_config::sections::health_probe_policy::Settings as ProbePolicy;
use betterlite_router::deployment_identity::spec::DeploymentSpec;

use super::probe::{PreparedProbe, prepare_probe};
use super::support::{
    FakeModels, FakeProvider, Reply, cycle, deployment, deployment_with_info, number,
    plain_request, policy, policy_with_timeout, seeded_random, text, with_timeout,
};

fn prepared_timeout(deployment: &DeploymentSpec, probe_policy: &ProbePolicy) -> Duration {
    let models = FakeModels::default();
    let mut random = seeded_random();
    let PreparedProbe::Ready { timeout, .. } =
        prepare_probe(deployment, &models, probe_policy, &mut random)
    else {
        panic!("the fixture deployment is probed");
    };
    timeout
}

#[tokio::test(start_paused = true)]
async fn an_override_times_out_its_own_probe() {
    let rows = [deployment_with_info(
        "leaf",
        "leaf-id",
        r#"{"model":"openai/leaf"}"#,
        r#"{"health_check_timeout":1}"#,
    )];
    let provider = FakeProvider::new()
        .with_reply("openai/leaf", Reply::DelayedSuccess(Duration::from_secs(5)));
    let result = cycle(
        plain_request(&rows),
        &provider,
        &FakeModels::default(),
        &policy(),
    )
    .await
    .expect("the cycle runs");
    let endpoint = result
        .unhealthy_endpoints
        .first()
        .expect("the timeout is reported");
    assert_eq!(endpoint.get("error"), Some(&text("Timeout exceeded")));
    assert_eq!(endpoint.get("exception_status"), Some(&number(408)));
    let exception = result
        .exceptions_by_model_id
        .get("leaf-id")
        .expect("the exception is filed under the model id");
    assert_eq!(exception.status_code, 408);
    assert_eq!(exception.message, "Timeout exceeded");
}

#[tokio::test(start_paused = true)]
async fn a_zero_override_keeps_the_sixty_second_default() {
    let rows = [deployment_with_info(
        "leaf",
        "leaf-id",
        r#"{"model":"openai/leaf"}"#,
        r#"{"health_check_timeout":0}"#,
    )];
    let provider = FakeProvider::new().with_reply(
        "openai/leaf",
        Reply::DelayedSuccess(Duration::from_secs(30)),
    );
    let result = cycle(
        plain_request(&rows),
        &provider,
        &FakeModels::default(),
        &policy(),
    )
    .await
    .expect("the cycle runs");
    assert_eq!(result.healthy_endpoints.len(), 1);
    assert!(result.unhealthy_endpoints.is_empty());
}

#[test]
fn the_policy_timeout_is_the_fallback() {
    let leaf = deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#);
    assert_eq!(
        prepared_timeout(&leaf, &policy_with_timeout(2)),
        Duration::from_secs(2)
    );
    assert_eq!(prepared_timeout(&leaf, &policy()), Duration::from_secs(60));
}

#[tokio::test(start_paused = true)]
async fn a_missing_override_times_out_at_the_policy_value() {
    let rows = [deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#)];
    let provider = FakeProvider::new()
        .with_reply("openai/leaf", Reply::DelayedSuccess(Duration::from_secs(3)));
    let result = cycle(
        plain_request(&rows),
        &provider,
        &FakeModels::default(),
        &policy_with_timeout(2),
    )
    .await
    .expect("the cycle runs");
    assert_eq!(result.unhealthy_endpoints.len(), 1);
}

#[test]
fn a_fractional_override_is_a_duration() {
    let leaf = with_timeout(
        deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#),
        0.5,
    );
    assert_eq!(
        prepared_timeout(&leaf, &policy()),
        Duration::from_millis(500)
    );
}

#[test]
fn a_negative_override_times_out_at_once() {
    let leaf = with_timeout(
        deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#),
        -1.0,
    );
    assert_eq!(prepared_timeout(&leaf, &policy()), Duration::ZERO);
}

#[test]
fn a_non_numeric_override_is_a_probe_failure() {
    let leaf = deployment_with_info(
        "leaf",
        "leaf-id",
        r#"{"model":"openai/leaf"}"#,
        r#"{"health_check_timeout":"soon"}"#,
    );
    let models = FakeModels::default();
    let probe_policy = policy();
    let mut random = seeded_random();
    let prepared = prepare_probe(&leaf, &models, &probe_policy, &mut random);
    let PreparedProbe::Failed(failure) = prepared else {
        panic!("a text timeout fails the probe");
    };
    assert_eq!(failure.status_code, 500);
    assert_eq!(
        failure.message,
        "model_info.health_check_timeout must be a number of seconds"
    );
}
