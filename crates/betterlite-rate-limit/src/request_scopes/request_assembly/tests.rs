#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::request_destination::RequestData;

use super::build_request_rate_limit_descriptors;
use crate::limiter_records::{
    RateLimitDescriptor, RateLimitDescriptorRateLimit, TagRateLimitResolver,
};
use crate::request_scopes::RequestScopeBuilder;
use crate::request_scopes::tests::support::{self, FixedRegistry, MapResolver, RecordingHealth};

struct Fixture {
    registry: FixedRegistry,
    health: RecordingHealth,
}

impl Fixture {
    fn healthy() -> Self {
        Self {
            registry: FixedRegistry(BTreeMap::new()),
            health: RecordingHealth::healthy(),
        }
    }

    fn failing() -> Self {
        Self {
            registry: FixedRegistry(BTreeMap::new()),
            health: RecordingHealth::failing(),
        }
    }

    fn builder<'tags>(
        &'tags self,
        tags: &'tags dyn TagRateLimitResolver,
    ) -> RequestScopeBuilder<'tags> {
        support::builder(60, &[("gpt4", "gpt-4")], &self.registry, &self.health, tags)
    }
}

async fn assemble(
    builder: &RequestScopeBuilder<'_>,
    auth: &UserAPIKeyAuth,
    data: &RequestData,
) -> Vec<RateLimitDescriptor> {
    build_request_rate_limit_descriptors(builder, auth, data, None)
        .await
        .expect("the request scopes build")
}

fn limits(descriptor: &RateLimitDescriptor) -> &RateLimitDescriptorRateLimit {
    descriptor
        .rate_limit
        .as_ref()
        .expect("the descriptor carries a rate limit")
}

fn keys(descriptors: &[RateLimitDescriptor]) -> Vec<&str> {
    descriptors
        .iter()
        .map(|descriptor| descriptor.key.as_str())
        .collect()
}

#[tokio::test]
async fn a_dynamic_key_asks_the_health_policy_for_the_model() {
    let fixture = Fixture::healthy();
    let tags = MapResolver::new(&[]);
    let builder = fixture.builder(&tags);
    let auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "metadata": {"rpm_limit_type": "dynamic"}}"#,
    );
    let data = support::data(r#"{"model": "gpt4"}"#);
    let descriptors = assemble(&builder, &auth, &data).await;
    assert_eq!(limits(&descriptors[0]).requests_per_unit, None);
    assert_eq!(fixture.health.models(), vec!["gpt-4".to_owned()]);
}

#[tokio::test]
async fn a_tpm_dynamic_key_asks_the_health_policy_for_the_model() {
    let fixture = Fixture::healthy();
    let tags = MapResolver::new(&[]);
    let builder = fixture.builder(&tags);
    let auth = support::auth(
        r#"{"api_key": "key-1", "tpm_limit": 20, "metadata": {"tpm_limit_type": "dynamic"}}"#,
    );
    let data = support::data(r#"{"model": "gpt4"}"#);
    let descriptors = assemble(&builder, &auth, &data).await;
    assert_eq!(limits(&descriptors[0]).tokens_per_unit, None);
    assert_eq!(fixture.health.models(), vec!["gpt-4".to_owned()]);
}

#[tokio::test]
async fn a_non_exact_dynamic_type_never_asks_the_health_policy() {
    let fixture = Fixture::failing();
    let tags = MapResolver::new(&[]);
    let builder = fixture.builder(&tags);
    let auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "metadata": {"rpm_limit_type": "DYNAMIC"}}"#,
    );
    let data = support::data(r#"{"model": "gpt4"}"#);
    let descriptors = assemble(&builder, &auth, &data).await;
    assert_eq!(limits(&descriptors[0]).requests_per_unit, Some(10));
    assert!(fixture.health.models().is_empty());
}

#[tokio::test]
async fn a_dynamic_key_enforces_when_the_health_policy_reports_failures() {
    let fixture = Fixture::failing();
    let tags = MapResolver::new(&[]);
    let builder = fixture.builder(&tags);
    let auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "metadata": {"rpm_limit_type": "dynamic"}}"#,
    );
    let data = support::data(r#"{"model": "gpt4"}"#);
    let descriptors = assemble(&builder, &auth, &data).await;
    assert_eq!(limits(&descriptors[0]).requests_per_unit, Some(10));
    assert_eq!(fixture.health.models(), vec!["gpt-4".to_owned()]);
}

#[tokio::test]
async fn a_static_key_never_asks_the_health_policy() {
    let fixture = Fixture::failing();
    let tags = MapResolver::new(&[]);
    let builder = fixture.builder(&tags);
    let auth = support::auth(r#"{"api_key": "key-1", "rpm_limit": 10}"#);
    let data = support::data(r#"{"model": "gpt4"}"#);
    let descriptors = assemble(&builder, &auth, &data).await;
    assert_eq!(limits(&descriptors[0]).requests_per_unit, Some(10));
    assert!(fixture.health.models().is_empty());
}

#[tokio::test]
async fn a_missing_model_never_asks_the_health_policy() {
    let fixture = Fixture::failing();
    let tags = MapResolver::new(&[]);
    let builder = fixture.builder(&tags);
    let auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "metadata": {"tpm_limit_type": "dynamic"}}"#,
    );
    let descriptors = assemble(&builder, &auth, &support::data("{}")).await;
    assert_eq!(limits(&descriptors[0]).requests_per_unit, Some(10));
    assert!(fixture.health.models().is_empty());
}

#[tokio::test]
async fn an_empty_model_never_asks_the_health_policy() {
    let fixture = Fixture::failing();
    let tags = MapResolver::new(&[]);
    let builder = fixture.builder(&tags);
    let auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "metadata": {"rpm_limit_type": "dynamic"}}"#,
    );
    let data = support::data(r#"{"model": ""}"#);
    let descriptors = assemble(&builder, &auth, &data).await;
    assert_eq!(limits(&descriptors[0]).requests_per_unit, None);
    assert!(fixture.health.models().is_empty());
}

#[tokio::test]
async fn a_non_text_model_never_asks_the_health_policy() {
    let fixture = Fixture::failing();
    let tags = MapResolver::new(&[]);
    let builder = fixture.builder(&tags);
    let auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "metadata": {"rpm_limit_type": "dynamic"}}"#,
    );
    let data = support::data(r#"{"model": 5}"#);
    let descriptors = assemble(&builder, &auth, &data).await;
    assert_eq!(limits(&descriptors[0]).requests_per_unit, None);
    assert!(fixture.health.models().is_empty());
}

fn assert_late_scope_order(descriptors: &[RateLimitDescriptor]) {
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
}

fn assert_key_scope_values(descriptors: &[RateLimitDescriptor]) {
    assert_eq!(support::descriptor_at(descriptors, 5).value, "key-1:gpt-4");
    assert_eq!(
        limits(support::descriptor_at(descriptors, 5)).requests_per_unit,
        Some(7)
    );
    assert_eq!(support::descriptor_at(descriptors, 6).value, "key-1:t1");
    assert_eq!(support::descriptor_at(descriptors, 7).value, "team-1:gpt-4");
    assert_eq!(
        limits(support::descriptor_at(descriptors, 7)).requests_per_unit,
        None
    );
    assert_eq!(
        limits(support::descriptor_at(descriptors, 7)).tokens_per_unit,
        Some(6)
    );
    assert_eq!(support::descriptor_at(descriptors, 8).value, "agent-1");
    assert_eq!(
        support::descriptor_at(descriptors, 9).value,
        "project-1:gpt-4"
    );
    assert_eq!(
        limits(support::descriptor_at(descriptors, 9)).requests_per_unit,
        Some(9)
    );
}

fn assert_project_org_tag_values(descriptors: &[RateLimitDescriptor]) {
    assert_eq!(
        support::descriptor_at(descriptors, 10).value,
        "project-1:gpt-4"
    );
    assert_eq!(
        limits(support::descriptor_at(descriptors, 10)).tokens_per_unit,
        Some(11)
    );
    assert_eq!(
        support::descriptor_at(descriptors, 11).value,
        "project-1:gpt-4"
    );
    assert_eq!(
        limits(support::descriptor_at(descriptors, 11)).tokens_per_unit,
        Some(12)
    );
    assert_eq!(support::descriptor_at(descriptors, 12).value, "org-1");
    assert_eq!(
        limits(support::descriptor_at(descriptors, 12)).requests_per_unit,
        Some(21)
    );
    assert_eq!(support::descriptor_at(descriptors, 13).value, "org-1:gpt-4");
    assert_eq!(
        limits(support::descriptor_at(descriptors, 13)).requests_per_unit,
        Some(13)
    );
    assert_eq!(support::descriptor_at(descriptors, 14).value, "t1");
    assert_eq!(
        limits(support::descriptor_at(descriptors, 14)).requests_per_unit,
        Some(8)
    );
}

#[tokio::test]
async fn the_assembly_appends_every_late_scope_in_order() {
    let registry = FixedRegistry(BTreeMap::new());
    let health = RecordingHealth::healthy();
    let tags = MapResolver::new(&[("t1", Some(8), None)]);
    let builder = support::builder(60, &[("gpt4", "gpt-4")], &registry, &health, &tags);
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
    let data = support::data(r#"{"model": "gpt4", "metadata": {"tags": ["t1"]}}"#);
    let descriptors = assemble(&builder, &auth, &data).await;
    assert_late_scope_order(&descriptors);
    assert_key_scope_values(&descriptors);
    assert_project_org_tag_values(&descriptors);
    assert_eq!(tags.calls(), [vec!["t1".to_owned()]]);
}

#[tokio::test]
async fn a_body_without_tags_never_asks_the_tag_resolver() {
    let fixture = Fixture::healthy();
    let tags = MapResolver::new(&[("t1", Some(8), None)]);
    let builder = fixture.builder(&tags);
    let auth = support::auth(r#"{"api_key": "key-1", "rpm_limit": 10}"#);
    let data = support::data(r#"{"model": "gpt4"}"#);
    let descriptors = assemble(&builder, &auth, &data).await;
    assert_eq!(keys(&descriptors), ["api_key", "model_per_key"]);
    assert!(tags.calls().is_empty());
}
