#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::fixtures;
use crate::auth_prefetch::entries::{
    AuthObjectRefs, MEMBERSHIP_AUTH_TTL, PrefetchEntry, iter_entries,
};
use crate::auth_prefetch::identity::{
    REGISTRY_ERROR_NEGATIVE_CACHE_TTL, identity_cache_keys, identity_memory_ttl,
};
use crate::auth_prefetch::rows::{RowKind, RowObject, validate_row};

use super::support::{Logs, json};

const MANAGEMENT_TTL: f64 = 60.0;
const ORGANIZATION_TTL: f64 = 7.0;

fn token(fields: &str) -> LiteLLMVerificationToken {
    serde_json::from_str(fields).expect("the token fixture builds")
}

fn decoded(text: &str) -> Json {
    serde_json::from_str(text).expect("the json fixture builds")
}

fn object(value: &Json) -> &IndexMap<String, Json> {
    match value {
        Json::Object(fields) => fields,
        other => panic!("the payload is not an object: {other:?}"),
    }
}

fn with_timestamp(row: JsonData, value: f64) -> JsonData {
    let JsonData::Object(mut fields) = row else {
        panic!("the fixture row is not an object");
    };
    fields.insert(
        "last_refreshed_at".to_owned(),
        JsonData::Number(Number::from_f64(value).expect("the timestamp is finite")),
    );
    JsonData::Object(fields)
}

fn payload_number(payload: &Json, field: &str) -> Option<f64> {
    match object(payload).get(field) {
        Some(Json::Number(value)) => value.as_f64(),
        Some(other) => panic!("the field is not a number: {other:?}"),
        None => None,
    }
}

fn full_refs() -> AuthObjectRefs {
    AuthObjectRefs {
        user_id: Some("user-1".to_owned()),
        team_id: Some("team-1".to_owned()),
        membership_user_id: Some("user-1".to_owned()),
        organization_id: Some("org-1".to_owned()),
        project_id: Some("project-1".to_owned()),
    }
}

#[test]
fn refs_all_none_defaults() {
    let refs = AuthObjectRefs::default();
    assert_eq!(refs, AuthObjectRefs::from_token(&token("{}")));
    assert!(refs.user_id.is_none());
    assert!(refs.team_id.is_none());
    assert!(refs.membership_user_id.is_none());
    assert!(refs.organization_id.is_none());
    assert!(refs.project_id.is_none());
}

#[test]
fn refs_org_id_projection() {
    let refs = AuthObjectRefs::from_token(&token(r#"{"org_id": "org-a"}"#));
    assert_eq!(refs.organization_id.as_deref(), Some("org-a"));
    assert!(refs.user_id.is_none());
    assert!(refs.team_id.is_none());
    assert!(refs.membership_user_id.is_none());
    assert!(refs.project_id.is_none());
}

#[test]
fn membership_requires_two_nonnull_ids() {
    let both = AuthObjectRefs::from_token(&token(r#"{"team_id": "team-1", "user_id": "user-1"}"#));
    assert_eq!(both.membership_user_id.as_deref(), Some("user-1"));
    let team_only = AuthObjectRefs::from_token(&token(r#"{"team_id": "team-1"}"#));
    assert!(team_only.membership_user_id.is_none());
    let user_only = AuthObjectRefs::from_token(&token(r#"{"user_id": "user-1"}"#));
    assert!(user_only.membership_user_id.is_none());
}

#[test]
fn empty_ids_still_referenced() {
    let refs = AuthObjectRefs::from_token(&token(r#"{"team_id": "", "user_id": ""}"#));
    assert_eq!(refs.user_id.as_deref(), Some(""));
    assert_eq!(refs.team_id.as_deref(), Some(""));
    assert_eq!(refs.membership_user_id.as_deref(), Some(""));
    let entries = iter_entries(&refs, MANAGEMENT_TTL, ORGANIZATION_TTL);
    let keys: Vec<&str> = entries.iter().map(|entry| entry.key.as_str()).collect();
    assert_eq!(keys, ["", "team_id:", "_", "team_membership::"]);
}

#[test]
fn seven_entry_order_and_key_formats() {
    let entries = iter_entries(&full_refs(), MANAGEMENT_TTL, ORGANIZATION_TTL);
    let expected = vec![
        PrefetchEntry {
            key: "user-1".to_owned(),
            row: RowKind::User,
            ttl: Some(MANAGEMENT_TTL),
        },
        PrefetchEntry {
            key: "team_id:team-1".to_owned(),
            row: RowKind::Team,
            ttl: Some(MANAGEMENT_TTL),
        },
        PrefetchEntry {
            key: "team-1_user-1".to_owned(),
            row: RowKind::Membership,
            ttl: Some(MEMBERSHIP_AUTH_TTL),
        },
        PrefetchEntry {
            key: "team_membership:user-1:team-1".to_owned(),
            row: RowKind::Membership,
            ttl: None,
        },
        PrefetchEntry {
            key: "org_id:org-1".to_owned(),
            row: RowKind::Organization,
            ttl: Some(ORGANIZATION_TTL),
        },
        PrefetchEntry {
            key: "org_id:org-1:with_budget".to_owned(),
            row: RowKind::Organization,
            ttl: Some(ORGANIZATION_TTL),
        },
        PrefetchEntry {
            key: "project_id:project-1".to_owned(),
            row: RowKind::Project,
            ttl: Some(MANAGEMENT_TTL),
        },
    ];
    assert_eq!(entries, expected);
}

#[test]
fn each_entry_ttl() {
    let entries = iter_entries(&full_refs(), MANAGEMENT_TTL, ORGANIZATION_TTL);
    let ttls: Vec<Option<f64>> = entries.into_iter().map(|entry| entry.ttl).collect();
    assert_eq!(
        ttls,
        [
            Some(MANAGEMENT_TTL),
            Some(MANAGEMENT_TTL),
            Some(5.0),
            None,
            Some(ORGANIZATION_TTL),
            Some(ORGANIZATION_TTL),
            Some(MANAGEMENT_TTL),
        ]
    );
    assert_eq!(MEMBERSHIP_AUTH_TTL, 5.0);
    assert_eq!(REGISTRY_ERROR_NEGATIVE_CACHE_TTL, 30.0);
}

#[test]
fn duplicate_entry_keys_retained() {
    let refs = AuthObjectRefs {
        user_id: Some("team_id:t1".to_owned()),
        team_id: Some("t1".to_owned()),
        membership_user_id: Some("team_id:t1".to_owned()),
        organization_id: None,
        project_id: None,
    };
    let entries = iter_entries(&refs, MANAGEMENT_TTL, ORGANIZATION_TTL);
    let keys: Vec<&str> = entries.iter().map(|entry| entry.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "team_id:t1",
            "team_id:t1",
            "t1_team_id:t1",
            "team_membership:team_id:t1:t1",
        ]
    );
    assert_eq!(entries[0].row, RowKind::User);
    assert_eq!(entries[1].row, RowKind::Team);
}

#[test]
fn identity_keys_hash_whole_api_key() {
    let keys = identity_cache_keys("Bearer sk-abc", None, false);
    assert_eq!(
        keys,
        [
            "0fd3043131f4b8cfe2a2eee5ff0c74d8453f5c87aa9696b23f76db906f26813f",
            "model_access_group_registry",
        ]
    );
    let empty = identity_cache_keys("", None, false);
    assert_eq!(
        empty,
        [
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "model_access_group_registry",
        ]
    );
}

#[test]
fn resolved_key_skips_hash() {
    let keys = identity_cache_keys("sk-abc", Some("end-1"), true);
    assert_eq!(
        keys,
        [
            "end_user_id:end-1",
            "end_user_restricted_registry",
            "model_access_group_registry",
        ]
    );
    assert!(!keys.contains(&betterlite_db::key_user_writes::hash::hash_token("sk-abc")));
}

#[test]
fn falsey_end_user_skips_both_keys() {
    let keys = identity_cache_keys("sk-abc", Some(""), false);
    assert_eq!(
        keys,
        [
            "1460db1b6902f8b1fc2a40d9381a24d0fd22c3bc1b2c6f999c521da73776fbe0",
            "model_access_group_registry",
        ]
    );
    let none = identity_cache_keys("sk-abc", None, false);
    assert_eq!(none, keys);
}

#[test]
fn group_registry_always_last() {
    let cases = [
        (false, None),
        (false, Some("end-1")),
        (true, None),
        (true, Some("end-1")),
    ];
    for (resolved, end_user) in cases {
        let keys = identity_cache_keys("sk-abc", end_user, resolved);
        assert_eq!(
            keys.last().map(String::as_str),
            Some("model_access_group_registry")
        );
    }
}

#[test]
fn string_value_ttl_min_30() {
    let value = decoded(r#""registry unavailable""#);
    assert_eq!(identity_memory_ttl(&value, 100.0), 30.0);
    assert_eq!(identity_memory_ttl(&value, 5.0), 5.0);
    assert_eq!(identity_memory_ttl(&value, 30.0), 30.0);
}

#[test]
fn nonstring_value_management_ttl() {
    for text in ["null", "false", "12", "[1]", r#"{"a": 1}"#] {
        assert_eq!(identity_memory_ttl(&decoded(text), 12.5), 12.5);
    }
}

#[test]
fn team_project_timestamp_override() {
    let team = with_timestamp(fixtures::team_row("team-1"), 1.5);
    let validated = validate_row(RowKind::Team, Some(&team), 42.0).expect("the team row validates");
    let RowObject::Team(row) = &validated else {
        panic!("the team row validates as a team");
    };
    assert_eq!(row.last_refreshed_at, Some(42.0));
    let payload = validated.payload().expect("the team payload");
    assert_eq!(payload_number(&payload, "last_refreshed_at"), Some(42.0));

    let project = with_timestamp(fixtures::project_row("project-1"), 1.5);
    let validated =
        validate_row(RowKind::Project, Some(&project), 9.5).expect("the project row validates");
    let RowObject::Project(row) = &validated else {
        panic!("the project row validates as a project");
    };
    assert_eq!(row.last_refreshed_at, Some(9.5));
    let payload = validated.payload().expect("the project payload");
    assert_eq!(payload_number(&payload, "last_refreshed_at"), Some(9.5));
}

#[test]
fn other_rows_not_timestamped() {
    let rows = [
        (
            RowKind::User,
            with_timestamp(fixtures::user_row("user-1"), 1.5),
        ),
        (
            RowKind::Membership,
            with_timestamp(fixtures::membership_row("user-1", "team-1"), 1.5),
        ),
        (
            RowKind::Organization,
            with_timestamp(fixtures::organization_row("org-1"), 1.5),
        ),
    ];
    for (kind, row) in rows {
        let validated = validate_row(kind, Some(&row), 42.0).expect("the row validates");
        let payload = validated.payload().expect("the payload");
        assert!(
            object(&payload).get("last_refreshed_at").is_none(),
            "{kind:?} carries no timestamp"
        );
    }
}

#[test]
fn null_row_silent_skip() {
    let (logs, _guard) = Logs::capture();
    for kind in [
        RowKind::User,
        RowKind::Team,
        RowKind::Membership,
        RowKind::Organization,
        RowKind::Project,
    ] {
        assert!(validate_row(kind, None, 42.0).is_none());
    }
    assert_eq!(logs.text(), "");
}

#[test]
fn invalid_row_warning_and_drop() {
    let (logs, _guard) = Logs::capture();
    let invalid = json(r#"{"team_id": 42}"#);
    assert!(validate_row(RowKind::Team, Some(&invalid), 42.0).is_none());
    let text = logs.text();
    assert!(text.contains("WARN"), "the drop is a warning: {text}");
    assert!(
        text.contains("auth prefetch: the row did not validate as its model"),
        "{text}"
    );
    assert!(text.contains("row=team_row"), "{text}");
    assert!(text.contains("model=LiteLLM_TeamTableCachedObj"), "{text}");
}
