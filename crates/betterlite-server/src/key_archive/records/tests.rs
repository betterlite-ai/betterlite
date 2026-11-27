#![cfg(test)]

use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::audit_events::identity::CallerIdentity;
use crate::key_archive::clock::{ArchiveClock, SystemArchiveClock};

use super::transform_verification_tokens_to_deleted_records;

const DELETED_AT: &str = "2026-05-06T07:08:09.123Z";

const SAMPLE_TOKEN: &str = r#"{
    "token": "sk-hashed-abc123",
    "key_name": "primary-key",
    "key_alias": "alias-one",
    "spend": 12.5,
    "total_spend": 40.25,
    "max_budget": 100.75,
    "expires": "2027-03-04T05:06:07.890Z",
    "models": ["gpt-4o", "claude-3"],
    "aliases": {"team": "alpha"},
    "config": {"region": "eu"},
    "user_id": "user-1",
    "team_id": "team-1",
    "agent_id": "agent-1",
    "project_id": "project-1",
    "max_parallel_requests": 7,
    "metadata": {"tier": "gold"},
    "tpm_limit": 1000,
    "rpm_limit": 500,
    "tpd_limit": 100000,
    "budget_duration": "30d",
    "budget_reset_at": "2027-01-02T00:00:00.500",
    "allowed_cache_controls": ["no-cache"],
    "allowed_routes": ["/chat/completions"],
    "key_type": "default",
    "permissions": {"manage_keys": true},
    "model_spend": {"gpt-4o": 3.25},
    "model_max_budget": {"gpt-4o": 50.5},
    "budget_fallbacks": {"gpt-4o": ["gpt-4o-mini"]},
    "soft_budget_cooldown": true,
    "blocked": false,
    "litellm_budget_table": {"budget_id": "budget-row"},
    "budget_id": "budget-1",
    "org_id": "org-1",
    "created_at": "2026-01-02T03:04:05.678",
    "created_by": "creator-1",
    "updated_at": "2026-02-03T04:05:06.789",
    "updated_by": "editor-1",
    "settings_updated_at": "2026-02-04T05:06:07.891",
    "last_active": "2026-02-05T06:07:08.912",
    "object_permission_id": "perm-1",
    "object_permission": {"object_permission_id": "perm-1"},
    "access_group_ids": ["group-1", "group-2"],
    "rotation_count": 3,
    "auto_rotate": true,
    "rotation_interval": "7d",
    "last_rotation_at": "2026-02-06T07:08:09.123",
    "key_rotation_at": "2026-02-07T08:09:10.234",
    "router_settings": {"num_retries": 2},
    "budget_limits": [{"budget_id": "budget-row"}]
}"#;

struct CountingClock {
    calls: AtomicUsize,
    instant: DateTime<Utc>,
}

impl ArchiveClock for CountingClock {
    fn now(&self) -> DateTime<Utc> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.instant
    }
}

fn deleted_at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(DELETED_AT)
        .expect("the clock instant parses")
        .with_timezone(&Utc)
}

fn counting_clock() -> CountingClock {
    CountingClock {
        calls: AtomicUsize::new(0),
        instant: deleted_at(),
    }
}

fn read_count(clock: &CountingClock) -> usize {
    clock.calls.load(Ordering::SeqCst)
}

fn token(json: &str) -> LiteLLMVerificationToken {
    serde_json::from_str(json).expect("the sample token parses")
}

fn caller() -> CallerIdentity {
    CallerIdentity {
        user_id: Some(String::from("deleter-1")),
        api_key: Some(SecretString::from("sk-caller-hash")),
        metadata: None,
        team_metadata: None,
    }
}

fn keys() -> [LiteLLMVerificationToken; 3] {
    [
        token(r#"{"token": "sk-a", "key_name": "key-a"}"#),
        token(r#"{"token": "sk-b", "key_name": "key-b"}"#),
        token(r#"{"token": "sk-c", "key_name": "key-c"}"#),
    ]
}

#[test]
fn empty_keys_return_no_records_and_do_not_read_the_clock() {
    let clock = counting_clock();
    let records = transform_verification_tokens_to_deleted_records(&[], &caller(), None, &clock);
    assert_eq!(records.len(), 0);
    assert_eq!(read_count(&clock), 0);
}

#[test]
fn records_share_one_deleted_at() {
    let clock = counting_clock();
    let records =
        transform_verification_tokens_to_deleted_records(&keys(), &caller(), None, &clock);
    assert_eq!(read_count(&clock), 1);
    let expected = Some(Timestamp::Naive(deleted_at().naive_utc()));
    records
        .iter()
        .for_each(|record| assert_eq!(record.deleted_at, expected));
}

#[test]
fn records_follow_the_key_order() {
    let clock = counting_clock();
    let records =
        transform_verification_tokens_to_deleted_records(&keys(), &caller(), None, &clock);
    let names: Vec<Option<String>> = records
        .iter()
        .map(|record| record.token.key_name.clone())
        .collect();
    assert_eq!(
        names,
        vec![
            Some(String::from("key-a")),
            Some(String::from("key-b")),
            Some(String::from("key-c"))
        ]
    );
}

#[test]
fn every_record_copies_its_token() {
    let clock = counting_clock();
    let original = token(SAMPLE_TOKEN);
    let records = transform_verification_tokens_to_deleted_records(
        std::slice::from_ref(&original),
        &caller(),
        Some("changer-1"),
        &clock,
    );
    assert_eq!(records.len(), 1);
    let mut expected = original;
    expected.org_id = None;
    expected.litellm_budget_table = None;
    expected.object_permission = None;
    expected.budget_limits = None;
    assert_eq!(
        serde_json::to_string(&records[0].token).expect("the archived token serializes"),
        serde_json::to_string(&expected).expect("the expected token serializes")
    );
}

#[test]
fn joined_relations_are_cleared() {
    let clock = counting_clock();
    let original = token(SAMPLE_TOKEN);
    assert!(original.litellm_budget_table.is_some());
    assert!(original.object_permission.is_some());
    assert!(original.budget_limits.is_some());
    let records = transform_verification_tokens_to_deleted_records(
        std::slice::from_ref(&original),
        &caller(),
        None,
        &clock,
    );
    assert_eq!(records[0].token.litellm_budget_table, None);
    assert_eq!(records[0].token.object_permission, None);
    assert_eq!(records[0].token.budget_limits, None);
}

#[test]
fn organization_id_receives_the_rows_org_id() {
    let clock = counting_clock();
    let records = transform_verification_tokens_to_deleted_records(
        &[token(r#"{"token": "sk-a", "org_id": "org-1"}"#)],
        &caller(),
        None,
        &clock,
    );
    assert_eq!(records[0].organization_id, Some(String::from("org-1")));
    assert_eq!(records[0].token.org_id, None);
}

#[test]
fn token_without_org_id_leaves_organization_id_unset() {
    let clock = counting_clock();
    let records = transform_verification_tokens_to_deleted_records(
        &[token(r#"{"token": "sk-a"}"#)],
        &caller(),
        None,
        &clock,
    );
    assert_eq!(records[0].organization_id, None);
    assert_eq!(records[0].token.org_id, None);
}

#[test]
fn deleted_by_names_the_caller() {
    let clock = counting_clock();
    let records = transform_verification_tokens_to_deleted_records(
        &[token(r#"{"token": "sk-a"}"#)],
        &caller(),
        None,
        &clock,
    );
    assert_eq!(records[0].deleted_by, Some(String::from("deleter-1")));
    let actor = records[0]
        .deleted_by_api_key
        .as_ref()
        .map(|secret| secret.expose_secret().as_str());
    assert_eq!(actor, Some("sk-caller-hash"));
}

#[test]
fn missing_caller_leaves_the_deletion_actors_unset() {
    let clock = counting_clock();
    let records = transform_verification_tokens_to_deleted_records(
        &[token(r#"{"token": "sk-a"}"#)],
        &CallerIdentity::default(),
        None,
        &clock,
    );
    assert_eq!(records[0].deleted_by, None);
    let actor = records[0]
        .deleted_by_api_key
        .as_ref()
        .map(|secret| secret.expose_secret().as_str());
    assert_eq!(actor, None);
}

#[test]
fn litellm_changed_by_is_copied_verbatim() {
    let clock = counting_clock();
    let records = transform_verification_tokens_to_deleted_records(
        &[token(r#"{"token": "sk-a"}"#)],
        &caller(),
        Some("header-admin"),
        &clock,
    );
    assert_eq!(
        records[0].litellm_changed_by,
        Some(String::from("header-admin"))
    );
}

#[test]
fn missing_litellm_changed_by_stays_unset() {
    let clock = counting_clock();
    let records = transform_verification_tokens_to_deleted_records(
        &[token(r#"{"token": "sk-a"}"#)],
        &caller(),
        None,
        &clock,
    );
    assert_eq!(records[0].litellm_changed_by, None);
}

#[test]
fn every_record_gets_a_fresh_uuid_id() {
    let clock = counting_clock();
    let records = transform_verification_tokens_to_deleted_records(
        &[token(r#"{"token": "sk-a"}"#), token(r#"{"token": "sk-b"}"#)],
        &caller(),
        None,
        &clock,
    );
    let ids: Vec<String> = records
        .iter()
        .filter_map(|record| record.id.clone())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    ids.iter().for_each(|id| {
        let parsed = Uuid::parse_str(id).expect("the archived id is a uuid");
        assert_eq!(parsed.get_version_num(), 4);
    });
}

#[test]
fn system_clock_reads_the_current_time() {
    let floor = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
        .expect("the floor parses")
        .with_timezone(&Utc);
    assert!(SystemArchiveClock.now() > floor);
}
