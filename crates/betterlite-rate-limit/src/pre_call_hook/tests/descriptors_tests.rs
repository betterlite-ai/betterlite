#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::request_destination::RequestData;

use super::support::{self, Fixture, Parts};
use crate::limiter_records::{RateLimitDescriptor, RateLimitDescriptorRateLimit};

fn parts() -> Parts {
    let mut parts = Parts::new(Vec::new(), Vec::new(), None);
    parts.groups = vec![(String::from("gpt4"), String::from("gpt-4"))];
    parts.tags = vec![(String::from("t1"), Some(8), None)];
    parts.agents = BTreeMap::from([(
        String::from("agent-1"),
        support::policy("agent-1", Some(14)),
    )]);
    parts
}

async fn descriptors(
    fixture: &Fixture,
    auth: &UserAPIKeyAuth,
    data: &RequestData,
) -> Vec<RateLimitDescriptor> {
    fixture
        .hook
        .build_request_rate_limit_descriptors(auth, data, Some("acompletion"))
        .await
        .expect("the descriptors build")
}

fn keys(descriptors: &[RateLimitDescriptor]) -> Vec<&str> {
    descriptors
        .iter()
        .map(|descriptor| descriptor.key.as_str())
        .collect()
}

fn limits(descriptor: &RateLimitDescriptor) -> &RateLimitDescriptorRateLimit {
    descriptor
        .rate_limit
        .as_ref()
        .expect("the descriptor carries a rate limit")
}

fn at(descriptors: &[RateLimitDescriptor], index: usize) -> &RateLimitDescriptor {
    descriptors.get(index).expect("the descriptor is present")
}

fn ordered_auth() -> UserAPIKeyAuth {
    let mut auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "user_id": "u1", "user_rpm_limit": 2,
            "team_id": "team-1", "team_rpm_limit": 3, "team_member_rpm_limit": 4,
            "end_user_id": "e1", "end_user_rpm_limit": 5, "agent_id": "agent-1",
            "org_id": "org-1", "organization_rpm_limit": 21, "project_id": "project-1",
            "metadata": {"model_rpm_limit": {"gpt-4": 7}, "tag_rpm_limit": {"t1": 8}},
            "team_metadata": {"model_tpm_limit": {"gpt-4": 6}},
            "project_metadata": {"model_rpm_limit": {"gpt-4": 9},
                                 "model_itpm_limit": {"gpt-4": 11},
                                 "model_otpm_limit": {"gpt-4": 12}},
            "organization_metadata": {"model_rpm_limit": {"gpt-4": 13}}}"#,
    );
    support::attach(
        &mut auth,
        r#"{"managed_agent_policy": {"agent_id": "agent-1", "agent_name": "agent-1",
                                      "agent_card_params": {}, "rpm_limit": 14}}"#,
    );
    auth
}

fn assert_litellm_order(descriptors: &[RateLimitDescriptor]) {
    assert_eq!(
        keys(descriptors),
        [
            "api_key",
            "user",
            "team",
            "team_member",
            "end_user",
            "model_per_key",
            "tag_per_key",
            "model_per_team",
            "agent",
            "model_per_project",
            "model_per_project_itpm",
            "model_per_project_otpm",
            "organization",
            "model_per_organization",
            "tag"
        ]
    );
    assert_eq!(at(descriptors, 5).value, "key-1:gpt-4");
    assert_eq!(limits(at(descriptors, 5)).requests_per_unit, Some(7));
    assert_eq!(at(descriptors, 10).value, "project-1:gpt-4");
    assert_eq!(limits(at(descriptors, 10)).tokens_per_unit, Some(11));
    assert_eq!(at(descriptors, 14).value, "t1");
}

#[tokio::test]
async fn descriptors_follow_the_litellm_order() {
    let fixture = Fixture::new(&parts());
    let auth = ordered_auth();
    let data = support::data(r#"{"model": "gpt4", "metadata": {"tags": ["t1"]}}"#);

    let descriptors = descriptors(&fixture, &auth, &data).await;

    assert_litellm_order(&descriptors);
}

#[tokio::test]
async fn descriptors_count_a_non_text_limit_type_and_model_as_absent() {
    let fixture = Fixture::new(&parts());
    let auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10,
            "metadata": {"rpm_limit_type": 5, "tpm_limit_type": null}}"#,
    );
    let data = support::data(r#"{"model": 5}"#);

    let descriptors = descriptors(&fixture, &auth, &data).await;

    assert_eq!(keys(&descriptors), ["api_key"]);
    assert_eq!(limits(at(&descriptors, 0)).requests_per_unit, Some(10));
    assert!(fixture.health.models().is_empty());
}

#[tokio::test]
async fn descriptors_ask_the_health_policy_for_a_named_dynamic_model() {
    let fixture = Fixture::new(&parts());
    let auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "metadata": {"rpm_limit_type": "dynamic"}}"#,
    );
    let data = support::data(r#"{"model": "gpt4"}"#);

    let descriptors = descriptors(&fixture, &auth, &data).await;

    assert_eq!(limits(at(&descriptors, 0)).requests_per_unit, None);
    assert_eq!(fixture.health.models(), vec![String::from("gpt4")]);
}

#[tokio::test]
async fn descriptors_skip_the_health_policy_without_a_text_model() {
    let fixture = Fixture::new(&parts());
    let auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "metadata": {"rpm_limit_type": "dynamic"}}"#,
    );
    let data = support::data(r#"{"model": 5}"#);

    let descriptors = descriptors(&fixture, &auth, &data).await;

    assert_eq!(keys(&descriptors), ["api_key"]);
    assert!(fixture.health.models().is_empty());
}
