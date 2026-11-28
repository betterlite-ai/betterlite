#![cfg(test)]

mod postgres;
mod support;

use std::slice::from_ref;
use std::sync::Arc;

use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use secrecy::ExposeSecret;

use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::bulk_team_removal::audit::{DeletedKeyAudit as _, KeyEventDeletedAudit};
use crate::bulk_team_removal::error::BulkRemovalError;
use crate::bulk_team_removal::service::BulkTeamRemoval;

const COMMIT_EVENTS: [&str; 18] = [
    "lookup:team-1",
    "transaction:u1",
    "lock:team-1",
    "roster:team-1",
    "users:u1;",
    "tokens:u1",
    "mappings:k1",
    "write-roster:team-1:1",
    "write-teams:u1:other",
    "delete-memberships:u1",
    "archive:k1",
    "delete-keys:u1",
    "audit:k1:admin",
    "key-cache:k1",
    "publish:k1",
    "memory:jwt_key_mapping:iss:sub:claim-k1",
    "mapping:jwt_key_mapping:iss:sub:claim-k1",
    "metric:team=team-1,team_alias=:1",
];

#[tokio::test]
async fn duplicate_removed_before_transaction() {
    let fixture = support::fixture_with_team(support::team(
        "team-1",
        &[support::member(Some("u1"), None)],
    ));
    let members = [support::identity_id("u1"), support::identity_id("u1")];
    let results = fixture
        .remove("team-1", &members, &support::proxy_admin())
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0].success);
    assert_eq!(results[0].error, None);
    assert!(!results[1].success);
    assert_eq!(
        results[1].error.as_deref(),
        Some("Duplicate member in request")
    );
    let events = fixture.log.events();
    let transactions = events
        .iter()
        .filter(|event| event.starts_with("transaction:"))
        .count();
    assert_eq!(transactions, 1);
    assert!(events.iter().any(|event| event == "transaction:u1"));
    assert!(fixture.session.roster_of("team-1").is_empty());
}

#[tokio::test]
async fn email_backfill_requires_user_team() {
    let fixture = support::fixture();
    let email_only = [support::member(None, Some("u1@example.com"))];
    fixture
        .session
        .add_team(support::team("team-1", &email_only));
    fixture
        .session
        .add_team(support::team("team-2", &email_only));
    fixture
        .session
        .add_user(support::user("u1", Some("u1@example.com"), &["team-1"]));
    let request = [support::identity_id("u1")];
    let results = fixture
        .remove("team-2", &request, &support::proxy_admin())
        .await
        .unwrap();
    assert!(!results[0].success);
    assert_eq!(results[0].error.as_deref(), Some("User not found in team"));
    assert_eq!(fixture.session.roster_of("team-2"), email_only.to_vec());
    let removal = fixture
        .remove("team-1", &request, &support::proxy_admin())
        .await
        .unwrap();
    assert!(removal[0].success);
    assert!(fixture.session.roster_of("team-1").is_empty());
}

#[tokio::test]
async fn exact_missing_team_problem() {
    let fixture = support::fixture();
    let error = fixture
        .remove(
            "team-404",
            &[support::identity_id("u1")],
            &support::proxy_admin(),
        )
        .await
        .unwrap_err();
    let BulkRemovalError::Problem(problem) = error else {
        panic!("a missing team is a problem");
    };
    assert_eq!(problem.0.status, 404);
    assert_eq!(problem.0.title, "Team not found");
    assert_eq!(problem.0.r#type, "urn:litellm:error:team-not-found");
    assert_eq!(problem.0.detail, "Team id=team-404 does not exist in db");
    assert_eq!(problem.0.allowed, None);
}

#[tokio::test]
async fn exact_forbidden_problem() {
    let team = support::team_with_org("team-1", "org-1", &[support::member(Some("u2"), None)]);
    let fixture = support::fixture_with_team(team);
    let error = fixture
        .remove(
            "team-1",
            &[support::identity_id("u2")],
            &support::member_auth("u1"),
        )
        .await
        .unwrap_err();
    let BulkRemovalError::Problem(problem) = error else {
        panic!("a denied caller is a problem");
    };
    assert_eq!(problem.0.status, 403);
    assert_eq!(problem.0.title, "Forbidden");
    assert_eq!(problem.0.r#type, "urn:litellm:error:forbidden");
    assert_eq!(
        problem.0.detail,
        "Call not allowed. User not proxy admin OR team admin OR org admin for this team. route='/management/v1/teams/team-1/members/bulk_delete'"
    );
    assert!(
        fixture
            .log
            .events()
            .iter()
            .all(|event| !event.starts_with("transaction:"))
    );
}

#[tokio::test]
async fn lookup_before_authorization() {
    let fixture = support::fixture();
    let error = fixture
        .remove(
            "team-missing",
            &[support::identity_id("u1")],
            &support::member_auth("u1"),
        )
        .await
        .unwrap_err();
    let BulkRemovalError::Problem(problem) = error else {
        panic!("the missing team wins over the denial");
    };
    assert_eq!(problem.0.status, 404);
    assert_eq!(problem.0.r#type, "urn:litellm:error:team-not-found");
    assert_eq!(
        problem.0.detail,
        "Team id=team-missing does not exist in db"
    );
}

#[tokio::test]
async fn result_original_identity_and_index() {
    let fixture = support::fixture_with_team(support::team(
        "team-1",
        &[
            support::member(Some("u1"), None),
            support::member(Some("u2"), None),
            support::member(Some("u3"), None),
        ],
    ));
    fixture
        .session
        .add_user(support::user("u2", Some("u2@example.com"), &["other"]));
    let members = [
        support::identity_id("u1"),
        support::identity(None, Some("u2@example.com")),
        support::identity_id("u3"),
    ];
    let results = fixture
        .remove("team-1", &members, &support::proxy_admin())
        .await
        .unwrap();
    assert_eq!(results.len(), 3);
    assert!(results[0].success);
    assert_eq!(results[0].user_id.as_deref(), Some("u1"));
    assert_eq!(results[0].user_email, None);
    assert!(!results[1].success);
    assert_eq!(results[1].user_id, None);
    assert_eq!(results[1].user_email.as_deref(), Some("u2@example.com"));
    assert_eq!(results[1].error.as_deref(), Some("User not found in team"));
    assert!(results[2].success);
    assert_eq!(results[2].user_id.as_deref(), Some("u3"));
    assert_eq!(
        fixture.session.roster_of("team-1"),
        vec![support::member(Some("u2"), None)]
    );
}

#[tokio::test]
async fn missing_logger_noop() {
    let fixture = support::fixture_with_team(support::team(
        "team-1",
        &[support::member(Some("u1"), None)],
    ));
    let access = fixture.team_access();
    let service: BulkTeamRemoval<'_, support::LogGauge> = BulkTeamRemoval {
        session: &fixture.session,
        access: &access,
        jwt: &fixture.jwt,
        audit: &fixture.audit,
        invalidation: &fixture.invalidation,
        gauge: None,
        labels: fixture.labels(),
    };
    let results = service
        .bulk_remove_team_members(
            "team-1",
            &[support::identity_id("u1")],
            &support::proxy_admin(),
        )
        .await
        .unwrap();
    assert!(results[0].success);
    assert!(
        fixture
            .log
            .events()
            .iter()
            .all(|event| !event.starts_with("metric:"))
    );
}

#[tokio::test]
async fn metric_exception_debug_only() {
    let fixture = support::fixture_with_team(support::team(
        "team-1",
        &[support::member(Some("u1"), None)],
    ));
    fixture.session.add_key(support::key("k1", "team-1", "u1"));
    fixture.jwt.add_mapping("k1", "sub", "claim-k1", "iss");
    let context = support::mismatched_context(&fixture.settings);
    let access = fixture.team_access();
    let service = BulkTeamRemoval {
        session: &fixture.session,
        access: &access,
        jwt: &fixture.jwt,
        audit: &fixture.audit,
        invalidation: &fixture.invalidation,
        gauge: Some(&fixture.gauge),
        labels: LabelSource::Context(&context),
    };
    let (guard, captured) = support::capture_log();
    let results = service
        .bulk_remove_team_members(
            "team-1",
            &[support::identity_id("u1")],
            &support::proxy_admin(),
        )
        .await
        .unwrap();
    drop(guard);
    assert!(results[0].success);
    let events = fixture.log.events();
    assert!(events.iter().any(|event| event.starts_with("audit:")));
    assert!(events.iter().any(|event| event.starts_with("mapping:")));
    assert!(events.iter().all(|event| !event.starts_with("metric:")));
    let text = support::logged_text(&captured);
    assert!(
        text.contains(
            "Prometheus: failed to emit team members metric: the label context was built from different enum values"
        ),
        "{text}"
    );
}

#[tokio::test]
async fn writes_commit_in_source_order() {
    let fixture = support::fixture_with_team(support::team(
        "team-1",
        &[
            support::member(Some("u1"), None),
            support::member(Some("u2"), None),
        ],
    ));
    fixture
        .session
        .add_user(support::user("u1", None, &["team-1", "other", "team-1"]));
    fixture.session.add_membership("team-1", "u1");
    fixture.session.add_key(support::key("k1", "team-1", "u1"));
    fixture.jwt.add_mapping("k1", "sub", "claim-k1", "iss");
    let results = fixture
        .remove(
            "team-1",
            &[support::identity_id("u1")],
            &support::proxy_admin(),
        )
        .await
        .unwrap();
    assert!(results[0].success);
    assert_eq!(fixture.log.events(), support::lines(&COMMIT_EVENTS));
}

#[tokio::test]
async fn stale_array_only_member_matches_and_loses_the_team() {
    let fixture = support::fixture_with_team(support::team(
        "team-1",
        &[support::member(Some("u2"), None)],
    ));
    fixture
        .session
        .add_user(support::user("u1", None, &["team-1", "other", "team-1"]));
    fixture.session.add_membership("team-1", "u1");
    fixture.session.add_key(support::key("k1", "team-1", "u1"));
    let results = fixture
        .remove(
            "team-1",
            &[support::identity_id("u1")],
            &support::proxy_admin(),
        )
        .await
        .unwrap();
    assert!(results[0].success);
    assert_eq!(
        fixture.session.roster_of("team-1"),
        vec![support::member(Some("u2"), None)]
    );
    assert_eq!(fixture.session.user_teams("u1"), support::lines(&["other"]));
    assert!(fixture.session.memberships().is_empty());
    assert!(fixture.session.key_tokens().is_empty());
    assert_eq!(fixture.session.archived_tokens(), support::lines(&["k1"]));
    assert!(
        fixture
            .log
            .events()
            .iter()
            .all(|event| !event.starts_with("write-roster"))
    );
}

#[tokio::test]
async fn explicit_email_is_never_replaced() {
    let fixture = support::fixture_with_team(support::team(
        "team-1",
        &[support::member(None, Some("row@example.com"))],
    ));
    fixture
        .session
        .add_user(support::user("u1", Some("row@example.com"), &["team-1"]));
    let request = [support::identity(Some("u1"), Some("explicit@example.com"))];
    let results = fixture
        .remove("team-1", &request, &support::proxy_admin())
        .await
        .unwrap();
    assert!(results[0].success);
    assert_eq!(
        results[0].user_email.as_deref(),
        Some("explicit@example.com")
    );
    assert_eq!(
        fixture.session.roster_of("team-1"),
        vec![support::member(None, Some("row@example.com"))]
    );
    assert!(fixture.session.user_teams("u1").is_empty());
}

#[tokio::test]
async fn a_removal_without_keys_skips_archive_and_audit() {
    let fixture = support::fixture_with_team(support::team(
        "team-1",
        &[support::member(Some("u1"), None)],
    ));
    let results = fixture
        .remove(
            "team-1",
            &[support::identity_id("u1")],
            &support::proxy_admin(),
        )
        .await
        .unwrap();
    assert!(results[0].success);
    assert!(fixture.session.roster_of("team-1").is_empty());
    assert!(fixture.session.memberships().is_empty());
    let events = fixture.log.events();
    assert!(events.iter().all(|event| {
        !event.starts_with("archive:")
            && !event.starts_with("audit:")
            && !event.starts_with("key-cache:")
            && !event.starts_with("publish:")
            && !event.starts_with("memory:")
            && !event.starts_with("mapping:")
            && !event.starts_with("mappings:")
    }));
    assert!(
        events
            .iter()
            .any(|event| event == "metric:team=team-1,team_alias=:0")
    );
}

#[test]
fn deleted_key_audit_matches_the_hook() {
    let scheduler = Arc::new(support::RecordingScheduler::default());
    let audit = support::key_event_audit(Arc::clone(&scheduler), true);
    let adapter = KeyEventDeletedAudit { audit: &audit };
    let key = support::key("k1", "team-1", "u1");
    adapter
        .schedule_deleted(from_ref(&key), &support::key_event_caller())
        .unwrap();
    let records = scheduler.records();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.action, AuditAction::Deleted);
    assert_eq!(record.table_name, AuditTable::Key);
    assert_eq!(record.object_id, "k1");
    assert_eq!(record.changed_by.as_deref(), Some("admin"));
    assert_eq!(
        record
            .changed_by_api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-admin")
    );
    let before = record.before_value.as_ref().unwrap();
    assert!(before.get("token").is_some());
    let updated = record.updated_values.as_ref().unwrap();
    assert!(updated.as_object().unwrap().is_empty());
}

#[test]
fn disabled_audit_schedules_nothing() {
    let scheduler = Arc::new(support::RecordingScheduler::default());
    let audit = support::key_event_audit(Arc::clone(&scheduler), false);
    let adapter = KeyEventDeletedAudit { audit: &audit };
    let key = support::key("k1", "team-1", "u1");
    adapter
        .schedule_deleted(from_ref(&key), &support::key_event_caller())
        .unwrap();
    assert!(scheduler.records().is_empty());
}

#[tokio::test]
async fn removed_member_without_user_row_loses_keys_and_membership() {
    let fixture = support::fixture_with_team(support::team(
        "team-1",
        &[support::member(Some("u1"), None)],
    ));
    fixture.session.add_membership("team-1", "u1");
    fixture.session.add_key(support::key("k1", "team-1", "u1"));
    let results = fixture
        .remove(
            "team-1",
            &[support::identity_id("u1")],
            &support::proxy_admin(),
        )
        .await
        .unwrap();
    assert!(results[0].success);
    assert!(fixture.session.roster_of("team-1").is_empty());
    assert!(fixture.session.memberships().is_empty());
    assert!(fixture.session.key_tokens().is_empty());
    assert_eq!(fixture.session.archived_tokens(), support::lines(&["k1"]));
}

#[tokio::test]
async fn removed_roster_id_fetches_its_own_user_row() {
    let fixture = support::fixture_with_team(support::team(
        "team-1",
        &[support::member(Some("u2"), Some("e@example.com"))],
    ));
    fixture
        .session
        .add_user(support::user("u2", None, &["team-1"]));
    let request = [support::identity(None, Some("e@example.com"))];
    let results = fixture
        .remove("team-1", &request, &support::proxy_admin())
        .await
        .unwrap();
    assert!(results[0].success);
    assert!(fixture.session.user_teams("u2").is_empty());
    assert!(fixture.session.roster_of("team-1").is_empty());
}

#[tokio::test]
async fn empty_email_request_is_not_backfilled_from_a_user_row() {
    let fixture = support::fixture_with_team(support::team(
        "team-1",
        &[support::member(Some("u2"), None)],
    ));
    fixture
        .session
        .add_user(support::user("u1", Some(""), &["team-1"]));
    let request = [support::identity(None, Some(""))];
    let results = fixture
        .remove("team-1", &request, &support::proxy_admin())
        .await
        .unwrap();
    assert!(!results[0].success);
    assert_eq!(results[0].error.as_deref(), Some("User not found in team"));
    assert_eq!(
        fixture.session.user_teams("u1"),
        support::lines(&["team-1"])
    );
}

#[tokio::test]
async fn email_only_request_matches_a_stale_user_row() {
    let fixture = support::fixture_with_team(support::team(
        "team-1",
        &[support::member(Some("u2"), None)],
    ));
    fixture
        .session
        .add_user(support::user("u1", Some("u1@example.com"), &["team-1"]));
    let request = [support::identity(None, Some("u1@example.com"))];
    let results = fixture
        .remove("team-1", &request, &support::proxy_admin())
        .await
        .unwrap();
    assert!(results[0].success);
    assert!(fixture.session.user_teams("u1").is_empty());
}

#[tokio::test]
async fn postgres_concurrent_roster_delete() {
    Box::pin(postgres::concurrent_roster_delete()).await;
}

#[tokio::test]
async fn postgres_delete_rollback() {
    postgres::delete_rollback().await;
}

#[tokio::test]
async fn postgres_post_commit_effects() {
    postgres::post_commit_effects().await;
}
