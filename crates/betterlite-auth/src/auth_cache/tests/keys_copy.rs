use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use super::support::{local_cache, lock, recording_cache};
use crate::auth_cache::WriteOptions;
use crate::auth_cache::copy::copy_for_cache;
use crate::auth_cache::keys::{
    CacheKey, END_USER_RESTRICTED_OVERFLOW, GROUP_REGISTRY_OVERFLOW, NO_TEAM_MEMBERSHIP, Partition,
    TAG_REGISTRY_OVERFLOW, USER_NO_MCP_PERMISSION, partition,
};
use crate::resolved_key::auth::{AuthInput, ParentOtelSpan, UserAPIKeyAuth};

pub(crate) fn auth_fixture() -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).expect("the auth fixture builds")
}

pub(crate) fn payload(text: &str) -> Payload {
    Payload(BTreeMap::from([(
        "amount".to_owned(),
        JsonData::String(text.to_owned()),
    )]))
}

#[test]
fn bare_lower_hex_only() {
    assert_eq!(partition(&"a".repeat(64)), Partition::KeyObject);
    assert_eq!(partition(&"0".repeat(64)), Partition::KeyObject);
    assert_eq!(
        partition(&"0123456789abcdef".repeat(4)),
        Partition::KeyObject
    );
}

#[test]
fn uppercase_prefix_newline_and_length_not_key() {
    assert_eq!(partition(&"A".repeat(64)), Partition::Ordinary);
    assert_eq!(partition(&"a".repeat(63)), Partition::Ordinary);
    assert_eq!(partition(&"a".repeat(65)), Partition::Ordinary);
    assert_eq!(
        partition(&format!("hashed-jwt-{}", "a".repeat(64))),
        Partition::Ordinary
    );
    assert_eq!(
        partition(&format!("{}\n", "a".repeat(63))),
        Partition::Ordinary
    );
    assert_eq!(partition(&"g".repeat(64)), Partition::Ordinary);
}

#[test]
fn hex_management_id_uses_key_partition() {
    let identifier = "0123456789abcdef".repeat(4);
    assert_eq!(partition(&identifier), Partition::KeyObject);
    let fixture = recording_cache();
    fixture
        .cache
        .set(
            Some(&identifier),
            &Json::Bool(true),
            WriteOptions::default(),
        )
        .unwrap();
    let prefixed = CacheKey::ObjectPermissionId(&identifier).render();
    fixture
        .cache
        .set(Some(&prefixed), &Json::Bool(true), WriteOptions::default())
        .unwrap();
    assert_eq!(fixture.operations(), ["key:set", "ordinary:set"]);
}

#[test]
fn permission_link_and_row_keys() {
    assert_eq!(
        CacheKey::UserObjectPermissionId("user-1").render(),
        "user_object_permission_id:user-1"
    );
    assert_eq!(
        CacheKey::ObjectPermissionId("perm-1").render(),
        "object_permission_id:perm-1"
    );
}

#[test]
fn tag_keys_and_overflow() {
    assert_eq!(CacheKey::Tag("prod").render(), "tag:prod");
    assert_eq!(CacheKey::TagRegistry.render(), "tag_registry");
    assert_eq!(TAG_REGISTRY_OVERFLOW, "__tag_registry_overflow__");
}

#[test]
fn access_group_keys_counter_and_overflow() {
    assert_eq!(
        CacheKey::ModelAccessGroup("group-1").render(),
        "model_access_group:group-1"
    );
    assert_eq!(
        CacheKey::ModelAccessGroupRegistry.render(),
        "model_access_group_registry"
    );
    assert_eq!(
        CacheKey::ModelAccessGroupSpend("group-1").render(),
        "spend:model_access_group:group-1"
    );
    assert_eq!(
        GROUP_REGISTRY_OVERFLOW,
        "__model_access_group_registry_overflow__"
    );
}

#[test]
fn project_keys_and_counter() {
    assert_eq!(
        CacheKey::ProjectId("project-1").render(),
        "project_id:project-1"
    );
    assert_eq!(
        CacheKey::ProjectSpend("project-1").render(),
        "spend:project:project-1"
    );
}

#[test]
fn end_user_keys_registry_and_overflow() {
    assert_eq!(
        CacheKey::EndUserId("end-user-1").render(),
        "end_user_id:end-user-1"
    );
    assert_eq!(
        CacheKey::EndUserRestrictedRegistry.render(),
        "end_user_restricted_registry"
    );
    assert_eq!(
        END_USER_RESTRICTED_OVERFLOW,
        "__end_user_restricted_registry_overflow__"
    );
}

#[test]
fn membership_auth_and_reservation_keys() {
    assert_eq!(
        CacheKey::TeamMembershipAuth {
            team_id: "team-1",
            user_id: "user-1"
        }
        .render(),
        "team-1_user-1"
    );
    assert_eq!(
        CacheKey::TeamMembershipReservation {
            user_id: "user-1",
            team_id: "team-1"
        }
        .render(),
        "team_membership:user-1:team-1"
    );
}

#[test]
fn no_permission_and_no_membership_sentinels() {
    assert_eq!(USER_NO_MCP_PERMISSION, "__user_no_mcp_permission__");
    assert_eq!(NO_TEAM_MEMBERSHIP, "__no_team_membership__");
    let cache = local_cache(Some(60.0_f64));
    let key = CacheKey::TeamMembershipReservation {
        user_id: "user-1",
        team_id: "team-1",
    }
    .render();
    let written = Json::Text(NO_TEAM_MEMBERSHIP.to_owned());
    cache
        .set(Some(&key), &written, WriteOptions::default())
        .unwrap();
    assert_eq!(cache.get(&key, false), Some(written));
    assert_eq!(cache.get("team_membership:user-2:team-1", false), None);
}

#[test]
fn delimiters_and_empty_ids_not_escaped() {
    assert_eq!(CacheKey::Tag("a:b").render(), "tag:a:b");
    assert_eq!(
        CacheKey::TeamMembershipAuth {
            team_id: "t:1",
            user_id: "u_1"
        }
        .render(),
        "t:1_u_1"
    );
    assert_eq!(
        CacheKey::TeamMembershipAuth {
            team_id: "",
            user_id: ""
        }
        .render(),
        "_"
    );
    assert_eq!(
        CacheKey::TeamMembershipReservation {
            user_id: "",
            team_id: ""
        }
        .render(),
        "team_membership::"
    );
    assert_eq!(CacheKey::Tag("").render(), "tag:");
    assert_eq!(CacheKey::ProjectId("").render(), "project_id:");
}

#[test]
fn management_ttl_only_none_falls_back() {
    let fixture = recording_cache();
    assert_eq!(
        fixture.cache.management_object_ttl().to_bits(),
        45.0_f64.to_bits()
    );
    lock(&fixture.ordinary).ttl = Some(0.0_f64);
    assert_eq!(
        fixture.cache.management_object_ttl().to_bits(),
        0.0_f64.to_bits()
    );
    lock(&fixture.ordinary).ttl = Some(-5.0_f64);
    assert_eq!(
        fixture.cache.management_object_ttl().to_bits(),
        (-5.0_f64).to_bits()
    );
    lock(&fixture.ordinary).ttl = Some(60.0_f64);
    assert_eq!(
        fixture.cache.management_object_ttl().to_bits(),
        60.0_f64.to_bits()
    );
    lock(&fixture.key_objects).ttl = Some(7.0_f64);
    assert_eq!(
        fixture.cache.management_object_ttl().to_bits(),
        60.0_f64.to_bits()
    );
}

fn filled_auth() -> UserAPIKeyAuth {
    let mut auth = auth_fixture();
    auth.extra.budget_reservation = Some(payload("reserved"));
    auth.extra.budget_throttle_pct = Some(Decimal::from(12_i64));
    auth.extra.parent_otel_span = Some(ParentOtelSpan::Json(JsonData::String("span".to_owned())));
    auth.extra.request_route = Some("/v1/chat/completions".to_owned());
    auth.extra.matched_model_access_groups = Some(vec!["group-1".to_owned()]);
    auth
}

#[test]
fn copy_clears_exactly_four_fields() {
    let auth = filled_auth();
    let copied = copy_for_cache(&auth);
    assert_eq!(copied.extra.budget_reservation, None);
    assert_eq!(copied.extra.budget_throttle_pct, None);
    assert!(copied.extra.parent_otel_span.is_none());
    assert_eq!(copied.extra.request_route, None);
    assert_eq!(
        copied.extra.matched_model_access_groups,
        Some(vec!["group-1".to_owned()])
    );
    assert_eq!(
        serde_json::to_value(&copied).unwrap(),
        serde_json::to_value(&auth).unwrap()
    );
}

#[test]
fn copy_original_unchanged() {
    let auth = filled_auth();
    let _copied = copy_for_cache(&auth);
    assert_eq!(auth.extra.budget_reservation, Some(payload("reserved")));
    assert_eq!(auth.extra.budget_throttle_pct, Some(Decimal::from(12_i64)));
    assert!(auth.extra.parent_otel_span.is_some());
    assert_eq!(
        auth.extra.request_route.as_deref(),
        Some("/v1/chat/completions")
    );
}

#[test]
fn copy_keeps_trusted_markers() {
    let mut auth = auth_fixture();
    auth.extra.requires_fresh_policy = true;
    auth.extra.provenance.via_virtual_key = true;
    auth.extra.provenance.authenticated_by_custom_auth = true;
    auth.extra.mcp_access.mcp_admitted_user_subject = true;
    auth.extra.mcp_access.mcp_explicit_grants_only = true;
    auth.extra.is_session_token = true;
    auth.managed_delegation_verified = true;
    let mut copied = copy_for_cache(&auth);
    assert!(copied.extra.requires_fresh_policy);
    assert!(copied.extra.provenance.via_virtual_key);
    assert!(copied.extra.provenance.authenticated_by_custom_auth);
    assert!(copied.extra.mcp_access.mcp_admitted_user_subject);
    assert!(copied.extra.mcp_access.mcp_explicit_grants_only);
    assert!(copied.extra.is_session_token);
    assert!(copied.take_delegation_verified());
}

#[test]
fn copy_preserves_nested_values() {
    let nested = JsonData::Object(BTreeMap::from([(
        "email".to_owned(),
        JsonData::String("user@example.com".to_owned()),
    )]));
    let mut auth = auth_fixture();
    auth.extra.user = Some(nested.clone());
    auth.extra.created_by_user = Some(nested.clone());
    auth.extra.jwt_claims = Some(payload("claim"));
    let copied = copy_for_cache(&auth);
    assert_eq!(copied.extra.user, Some(nested.clone()));
    assert_eq!(copied.extra.created_by_user, Some(nested));
    assert_eq!(copied.extra.jwt_claims, Some(payload("claim")));
    assert_eq!(
        serde_json::to_value(&copied).unwrap(),
        serde_json::to_value(&auth).unwrap()
    );
}
