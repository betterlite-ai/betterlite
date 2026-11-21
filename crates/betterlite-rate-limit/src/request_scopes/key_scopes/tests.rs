#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::request_destination::RequestData;
use rstest::rstest;

use super::{ScopeInputs, create_rate_limit_descriptors};
use crate::limiter_records::{RateLimitDescriptor, RateLimitDescriptorRateLimit};
use crate::request_scopes::RequestScopeBuilder;
use crate::request_scopes::tests::support::{self, FixedRegistry, MapResolver, NoRouter};

const WINDOW: i64 = 45;

struct Fixture {
    registry: FixedRegistry,
    health: NoRouter,
    tags: MapResolver,
}

impl Fixture {
    fn new() -> Self {
        Self {
            registry: FixedRegistry(BTreeMap::new()),
            health: NoRouter,
            tags: MapResolver::new(&[]),
        }
    }

    fn builder(&self, groups: &[(&str, &str)]) -> RequestScopeBuilder<'_> {
        support::builder(WINDOW, groups, &self.registry, &self.health, &self.tags)
    }
}

fn inputs<'request>(
    auth: &'request UserAPIKeyAuth,
    data: &'request RequestData,
    types: (Option<&'request str>, Option<&'request str>),
    model_has_failures: bool,
    call_type: Option<&'request str>,
) -> ScopeInputs<'request> {
    ScopeInputs {
        auth,
        data,
        rpm_limit_type: types.0,
        tpm_limit_type: types.1,
        model_has_failures,
        call_type,
    }
}

fn create(
    builder: &RequestScopeBuilder<'_>,
    auth: &UserAPIKeyAuth,
    data: &RequestData,
    model_has_failures: bool,
) -> Vec<RateLimitDescriptor> {
    create_rate_limit_descriptors(
        builder,
        &inputs(auth, data, (None, None), model_has_failures, None),
    )
    .expect("the key scopes build")
}

fn only(descriptors: &[RateLimitDescriptor]) -> &RateLimitDescriptor {
    assert_eq!(
        descriptors.len(),
        1,
        "expected one descriptor, got {descriptors:?}"
    );
    &descriptors[0]
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

const MCP_TOOL: Option<&str> = Some("call_mcp_tool");

#[test]
fn an_api_key_descriptor_needs_a_key() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(r#"{"rpm_limit": 10}"#);
    assert!(create(&builder, &auth, &support::data("{}"), false).is_empty());
}

#[test]
fn an_empty_api_key_adds_no_descriptor() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(r#"{"api_key": "", "rpm_limit": 10}"#);
    assert!(create(&builder, &auth, &support::data("{}"), false).is_empty());
}

#[test]
fn an_api_key_descriptor_needs_one_limit() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(r#"{"api_key": "key-1"}"#);
    assert!(create(&builder, &auth, &support::data("{}"), false).is_empty());
}

#[test]
fn an_api_key_descriptor_carries_the_key_limits_and_window() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "tpm_limit": 20, "max_parallel_requests": 3}"#,
    );
    let descriptors = create(&builder, &auth, &support::data("{}"), false);
    let descriptor = only(&descriptors);
    assert_eq!(descriptor.key, "api_key");
    assert_eq!(descriptor.value, "key-1");
    assert_eq!(limits(descriptor).requests_per_unit, Some(10));
    assert_eq!(limits(descriptor).tokens_per_unit, Some(20));
    assert_eq!(limits(descriptor).max_parallel_requests, Some(3));
    assert_eq!(limits(descriptor).window_size, Some(WINDOW));
}

#[test]
fn a_max_parallel_requests_limit_alone_adds_the_api_key_descriptor() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(r#"{"api_key": "key-1", "max_parallel_requests": 3}"#);
    let descriptors = create(&builder, &auth, &support::data("{}"), false);
    let descriptor = only(&descriptors);
    assert_eq!(limits(descriptor).requests_per_unit, None);
    assert_eq!(limits(descriptor).tokens_per_unit, None);
    assert_eq!(limits(descriptor).max_parallel_requests, Some(3));
}

#[test]
fn the_budget_throttle_scales_the_key_limits() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "tpm_limit": 21, "budget_throttle_pct": 0.5}"#,
    );
    let descriptors = create(&builder, &auth, &support::data("{}"), false);
    let descriptor = only(&descriptors);
    assert_eq!(limits(descriptor).requests_per_unit, Some(5));
    assert_eq!(limits(descriptor).tokens_per_unit, Some(10));
}

#[test]
fn the_budget_throttle_keeps_a_trickle_of_one() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth =
        support::auth(r#"{"api_key": "key-1", "rpm_limit": 1, "budget_throttle_pct": 0.01}"#);
    let descriptors = create(&builder, &auth, &support::data("{}"), false);
    assert_eq!(limits(only(&descriptors)).requests_per_unit, Some(1));
}

#[test]
fn a_dynamic_key_keeps_its_descriptor_when_the_model_is_healthy() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(r#"{"api_key": "key-1", "rpm_limit": 10, "tpm_limit": 20}"#);
    let data = support::data("{}");
    let descriptors = create_rate_limit_descriptors(
        &builder,
        &inputs(&auth, &data, (Some("dynamic"), None), false, None),
    )
    .expect("the key scopes build");
    let descriptor = only(&descriptors);
    assert_eq!(limits(descriptor).requests_per_unit, None);
    assert_eq!(limits(descriptor).tokens_per_unit, Some(20));
}

#[test]
fn a_dynamic_tpm_type_drops_the_tokens_when_the_model_is_healthy() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(r#"{"api_key": "key-1", "tpm_limit": 20}"#);
    let data = support::data("{}");
    let healthy = create_rate_limit_descriptors(
        &builder,
        &inputs(&auth, &data, (None, Some("dynamic")), false, None),
    )
    .expect("the key scopes build");
    assert_eq!(limits(only(&healthy)).tokens_per_unit, None);
    let failing = create_rate_limit_descriptors(
        &builder,
        &inputs(&auth, &data, (None, Some("dynamic")), true, None),
    )
    .expect("the key scopes build");
    assert_eq!(limits(only(&failing)).tokens_per_unit, Some(20));
}

#[test]
fn the_throttle_applies_before_the_dynamic_policy() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "tpm_limit": 20, "budget_throttle_pct": 0.5}"#,
    );
    let data = support::data("{}");
    let healthy = create_rate_limit_descriptors(
        &builder,
        &inputs(&auth, &data, (Some("dynamic"), None), false, None),
    )
    .expect("the key scopes build");
    assert_eq!(limits(only(&healthy)).requests_per_unit, None);
    let failing = create_rate_limit_descriptors(
        &builder,
        &inputs(&auth, &data, (Some("dynamic"), None), true, None),
    )
    .expect("the key scopes build");
    assert_eq!(limits(only(&failing)).requests_per_unit, Some(5));
}

#[test]
fn a_guaranteed_throughput_type_is_enforced_without_failures() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(r#"{"api_key": "key-1", "rpm_limit": 10}"#);
    let data = support::data("{}");
    let descriptors = create_rate_limit_descriptors(
        &builder,
        &inputs(
            &auth,
            &data,
            (Some("guaranteed_throughput"), None),
            false,
            None,
        ),
    )
    .expect("the key scopes build");
    assert_eq!(limits(only(&descriptors)).requests_per_unit, Some(10));
}

#[rstest]
#[case::no_user_id(r#"{"user_rpm_limit": 4}"#)]
#[case::empty_user_id(r#"{"user_id": "", "user_rpm_limit": 4}"#)]
#[case::empty_team_member_user(r#"{"user_id": "", "team_member_rpm_limit": 5}"#)]
#[case::user_without_limit(r#"{"user_id": "u1"}"#)]
#[case::no_team_id(r#"{"team_rpm_limit": 4}"#)]
#[case::empty_team_id(r#"{"team_id": "", "team_rpm_limit": 4}"#)]
#[case::team_without_limit(r#"{"team_id": "team-1"}"#)]
#[case::empty_end_user_id(r#"{"end_user_id": "", "end_user_rpm_limit": 4}"#)]
#[case::no_end_user_id(r#"{"end_user_rpm_limit": 4}"#)]
fn the_identity_descriptors_need_a_non_empty_id_and_a_limit(#[case] wire: &str) {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let data = support::data("{}");
    let auth = support::auth(wire);
    assert!(create(&builder, &auth, &data, false).is_empty(), "{wire}");
}

#[test]
fn a_user_descriptor_carries_the_user_limits() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(r#"{"user_id": "u1", "user_rpm_limit": 4, "user_tpm_limit": 0}"#);
    let descriptors = create(&builder, &auth, &support::data("{}"), false);
    let descriptor = only(&descriptors);
    assert_eq!(descriptor.key, "user");
    assert_eq!(descriptor.value, "u1");
    assert_eq!(limits(descriptor).requests_per_unit, Some(4));
    assert_eq!(limits(descriptor).tokens_per_unit, Some(0));
    assert_eq!(limits(descriptor).window_size, Some(WINDOW));
}

#[test]
fn a_team_descriptor_carries_the_team_limits() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(r#"{"team_id": "team-1", "team_rpm_limit": 0, "team_tpm_limit": 6}"#);
    let descriptors = create(&builder, &auth, &support::data("{}"), false);
    let descriptor = only(&descriptors);
    assert_eq!(descriptor.key, "team");
    assert_eq!(descriptor.value, "team-1");
    assert_eq!(limits(descriptor).requests_per_unit, Some(0));
    assert_eq!(limits(descriptor).tokens_per_unit, Some(6));
    assert_eq!(limits(descriptor).window_size, Some(WINDOW));
}

#[test]
fn an_end_user_descriptor_carries_the_end_user_limits() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(r#"{"end_user_id": "e1", "end_user_rpm_limit": 5}"#);
    let descriptors = create(&builder, &auth, &support::data("{}"), false);
    let descriptor = only(&descriptors);
    assert_eq!(descriptor.key, "end_user");
    assert_eq!(descriptor.value, "e1");
    assert_eq!(limits(descriptor).requests_per_unit, Some(5));
    assert_eq!(limits(descriptor).tokens_per_unit, None);
    assert_eq!(limits(descriptor).window_size, Some(WINDOW));
}

#[test]
fn a_team_member_descriptor_needs_a_user_id_but_not_a_team() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(r#"{"team_id": "team-1", "team_member_rpm_limit": 5}"#);
    assert!(create(&builder, &auth, &support::data("{}"), false).is_empty());
}

#[test]
fn a_team_member_value_is_the_team_and_the_user() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(
        r#"{"user_id": "u1", "team_id": "team-1", "team_member_rpm_limit": 5,
            "team_member_tpm_limit": 7}"#,
    );
    let descriptors = create(&builder, &auth, &support::data("{}"), false);
    let descriptor = only(&descriptors);
    assert_eq!(descriptor.key, "team_member");
    assert_eq!(descriptor.value, "team-1:u1");
    assert_eq!(limits(descriptor).requests_per_unit, Some(5));
    assert_eq!(limits(descriptor).tokens_per_unit, Some(7));
}

#[test]
fn a_team_member_without_a_team_reads_none_as_its_prefix() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(r#"{"user_id": "u1", "team_member_rpm_limit": 5}"#);
    let descriptors = create(&builder, &auth, &support::data("{}"), false);
    assert_eq!(only(&descriptors).value, "None:u1");
}

#[test]
fn a_non_text_model_adds_no_model_descriptor() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[("gpt4", "gpt-4")]);
    let auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "team_id": "team-1",
            "metadata": {"model_rpm_limit": {"gpt-4": 7}},
            "team_metadata": {"model_tpm_limit": {"gpt-4": 6}}}"#,
    );
    let data = support::data(r#"{"model": 5}"#);
    let descriptors = create(&builder, &auth, &data, false);
    assert_eq!(keys(&descriptors), ["api_key"]);
}

#[test]
fn the_key_owned_descriptors_follow_the_handlers_order() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "user_id": "u1", "user_rpm_limit": 2,
            "team_id": "team-1", "team_rpm_limit": 3, "team_member_rpm_limit": 4,
            "end_user_id": "e1", "end_user_rpm_limit": 5}"#,
    );
    let descriptors = create(&builder, &auth, &support::data("{}"), false);
    assert_eq!(
        keys(&descriptors),
        ["api_key", "user", "team", "team_member", "end_user"]
    );
}

fn assert_every_builder_order(descriptors: &[RateLimitDescriptor]) {
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
            "mcp_per_key",
            "mcp_per_team",
            "mcp_per_team",
            "model_per_team",
            "agent",
            "agent_session",
            "agent"
        ]
    );
}

fn assert_model_tag_and_mcp_values(descriptors: &[RateLimitDescriptor]) {
    assert_eq!(support::descriptor_at(descriptors, 5).value, "key-1:gpt-4");
    assert_eq!(
        limits(support::descriptor_at(descriptors, 5)).requests_per_unit,
        Some(7)
    );
    assert_eq!(support::descriptor_at(descriptors, 6).value, "key-1:t1");
    assert_eq!(
        limits(support::descriptor_at(descriptors, 6)).requests_per_unit,
        Some(8)
    );
    assert_eq!(support::descriptor_at(descriptors, 7).value, "key-1:server");
    assert_eq!(
        limits(support::descriptor_at(descriptors, 7)).requests_per_unit,
        Some(9)
    );
    assert_eq!(
        support::descriptor_at(descriptors, 8).value,
        "team-1:server"
    );
    assert_eq!(
        limits(support::descriptor_at(descriptors, 8)).requests_per_unit,
        Some(5)
    );
    assert_eq!(
        support::descriptor_at(descriptors, 9).value,
        "team-2:server"
    );
    assert_eq!(
        limits(support::descriptor_at(descriptors, 9)).requests_per_unit,
        Some(7)
    );
}

fn assert_agent_values(descriptors: &[RateLimitDescriptor]) {
    assert_eq!(
        support::descriptor_at(descriptors, 10).value,
        "team-1:gpt-4"
    );
    assert_eq!(
        limits(support::descriptor_at(descriptors, 10)).tokens_per_unit,
        Some(6)
    );
    assert_eq!(support::descriptor_at(descriptors, 11).key, "agent");
    assert_eq!(support::descriptor_at(descriptors, 11).value, "agent-1");
    assert_eq!(
        limits(support::descriptor_at(descriptors, 11)).requests_per_unit,
        Some(11)
    );
    assert_eq!(support::descriptor_at(descriptors, 12).value, "agent-1:s1");
    assert_eq!(
        limits(support::descriptor_at(descriptors, 12)).requests_per_unit,
        Some(4)
    );
    assert_eq!(support::descriptor_at(descriptors, 13).value, "agent-2");
    assert_eq!(
        limits(support::descriptor_at(descriptors, 13)).requests_per_unit,
        Some(12)
    );
}

#[test]
fn every_builder_runs_in_the_handlers_order() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[("gpt4", "gpt-4")]);
    let mut auth = support::auth(
        r#"{"api_key": "key-1", "rpm_limit": 10, "user_id": "u1", "user_rpm_limit": 2,
            "team_id": "team-1", "team_rpm_limit": 3, "team_member_rpm_limit": 4,
            "end_user_id": "e1", "end_user_rpm_limit": 5, "agent_id": "agent-1",
            "metadata": {"model_rpm_limit": {"gpt-4": 7}, "tag_rpm_limit": {"t1": 8},
                         "mcp_rpm_limit": {"server": 9}},
            "team_metadata": {"model_tpm_limit": {"gpt-4": 6},
                              "mcp_rpm_limit": {"server": 5}}}"#,
    );
    support::attach(
        &mut auth,
        r#"{"managed_agent_policy": {"agent_id": "agent-1", "agent_name": "agent-1",
                                      "agent_card_params": {}, "rpm_limit": 11, "session_rpm_limit": 4},
            "invoked_agent_id": "agent-2",
            "invoked_agent_policy": {"agent_id": "agent-2", "agent_name": "agent-2",
                                      "agent_card_params": {}, "rpm_limit": 12},
            "source_team_rpm_limits": {"team-2": {"server": 7}}}"#,
    );
    let data = support::data(
        r#"{"model": "gpt4", "metadata": {"tags": ["t1"], "session_id": "s1"},
            "mcp_server_name": "server"}"#,
    );
    let descriptors = create_rate_limit_descriptors(
        &builder,
        &inputs(&auth, &data, (None, None), false, MCP_TOOL),
    )
    .expect("the key scopes build");
    assert_every_builder_order(&descriptors);
    assert_model_tag_and_mcp_values(&descriptors);
    assert_agent_values(&descriptors);
}

#[test]
fn the_mcp_builders_are_skipped_when_the_body_names_a_server_id() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth =
        support::auth(r#"{"api_key": "key-1", "metadata": {"mcp_rpm_limit": {"server": 9}}}"#);
    let data = support::data(r#"{"server_id": "srv-1", "mcp_server_name": "server"}"#);
    let descriptors = create_rate_limit_descriptors(
        &builder,
        &inputs(&auth, &data, (None, None), false, MCP_TOOL),
    )
    .expect("the key scopes build");
    assert!(descriptors.is_empty());
}

#[rstest]
#[case::absent(None)]
#[case::other_call_type(Some("acompletion"))]
fn the_mcp_builders_are_skipped_for_other_call_types(#[case] call_type: Option<&str>) {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth =
        support::auth(r#"{"api_key": "key-1", "metadata": {"mcp_rpm_limit": {"server": 9}}}"#);
    let data = support::data(r#"{"mcp_server_name": "server"}"#);
    let descriptors = create_rate_limit_descriptors(
        &builder,
        &inputs(&auth, &data, (None, None), false, call_type),
    )
    .expect("the key scopes build");
    assert!(descriptors.is_empty());
}

#[test]
fn the_mcp_builders_need_a_server_name() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let auth =
        support::auth(r#"{"api_key": "key-1", "metadata": {"mcp_rpm_limit": {"server": 9}}}"#);
    let data = support::data(r#"{"mcp_server_name": 5}"#);
    let descriptors = create_rate_limit_descriptors(
        &builder,
        &inputs(&auth, &data, (None, None), false, MCP_TOOL),
    )
    .expect("the key scopes build");
    assert!(descriptors.is_empty());
}

#[test]
fn an_agent_that_is_resolved_and_invoked_is_processed_once() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let mut auth = support::auth(r#"{"agent_id": "agent-1"}"#);
    support::attach(
        &mut auth,
        r#"{"managed_agent_policy": {"agent_id": "agent-1", "agent_name": "agent-1",
                                      "agent_card_params": {}, "rpm_limit": 11},
            "invoked_agent_id": "agent-1",
            "invoked_agent_policy": {"agent_id": "agent-1", "agent_name": "agent-1",
                                      "agent_card_params": {}, "rpm_limit": 12}}"#,
    );
    let descriptors = create(&builder, &auth, &support::data("{}"), false);
    let descriptor = only(&descriptors);
    assert_eq!(descriptor.key, "agent");
    assert_eq!(descriptor.value, "agent-1");
    assert_eq!(limits(descriptor).requests_per_unit, Some(11));
}

#[test]
fn a_metadata_agent_id_uses_the_invoked_policy() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let mut auth = support::auth("{}");
    support::attach(
        &mut auth,
        r#"{"invoked_agent_policy": {"agent_id": "agent-2", "agent_name": "agent-2",
                                      "agent_card_params": {}, "rpm_limit": 12}}"#,
    );
    let data = support::data(r#"{"metadata": {"agent_id": "agent-2"}}"#);
    let descriptors = create(&builder, &auth, &data, false);
    let descriptor = only(&descriptors);
    assert_eq!(descriptor.value, "agent-2");
    assert_eq!(limits(descriptor).requests_per_unit, Some(12));
}

#[test]
fn an_empty_agent_id_is_not_skipped() {
    let fixture = Fixture::new();
    let builder = fixture.builder(&[]);
    let mut auth = support::auth(r#"{"agent_id": ""}"#);
    support::attach(
        &mut auth,
        r#"{"managed_agent_policy": {"agent_id": "", "agent_name": "",
                                      "agent_card_params": {}, "rpm_limit": 13}}"#,
    );
    let descriptors = create(&builder, &auth, &support::data("{}"), false);
    assert_eq!(only(&descriptors).value, "");
}

#[test]
fn an_agent_without_a_policy_comes_from_the_registry() {
    let registry = FixedRegistry(BTreeMap::from([(
        "agent-1".to_owned(),
        support::policy("agent-1", Some(14)),
    )]));
    let health = NoRouter;
    let tags = MapResolver::new(&[]);
    let builder = support::builder(WINDOW, &[], &registry, &health, &tags);
    let auth = support::auth(r#"{"agent_id": "agent-1"}"#);
    let descriptors = create(&builder, &auth, &support::data("{}"), false);
    let descriptor = only(&descriptors);
    assert_eq!(descriptor.value, "agent-1");
    assert_eq!(limits(descriptor).requests_per_unit, Some(14));
}
