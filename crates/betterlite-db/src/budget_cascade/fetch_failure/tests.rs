#![cfg(test)]

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use tracing::subscriber::set_default;

use super::fetched_or_empty;
use crate::budget_cascade::collect_cascade::collect_budget_cascade;
use crate::budget_cascade::linked_rows::{KeyRow, OrganizationRow, TeamMembershipRow};
use crate::budget_cascade::tests::{
    Database, FakeRows, Logs, RecordingSchedule, budget, seed_linked_rows, spend_of, subscriber,
};
use crate::budget_cascade::{CounterReset, transaction::commit_budget_cascade};
use crate::budget_reset_policy::rollover::RolloverPolicy;

fn at() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("timestamp is valid")
}

const DISABLED: RolloverPolicy = RolloverPolicy { enabled: false };

#[tokio::test]
async fn a_failed_fetch_returns_no_rows_and_logs_the_warning() {
    let logs = Logs::default();
    let guard = set_default(subscriber(logs.clone()));
    let rows = fetched_or_empty("team memberships", || async {
        Err::<Vec<TeamMembershipRow>, String>("read failed".to_owned())
    })
    .await;
    drop(guard);
    assert!(rows.is_empty());
    let text = logs.text();
    assert!(text.contains("Failed to fetch linked rows for counter invalidation"));
    assert!(text.contains("team memberships"));
    assert!(text.contains("read failed"));
}

#[tokio::test]
async fn a_successful_fetch_returns_its_rows_without_a_warning() {
    let logs = Logs::default();
    let guard = set_default(subscriber(logs.clone()));
    let rows = fetched_or_empty("keys", || async {
        Ok::<Vec<KeyRow>, String>(vec![KeyRow {
            token: "token-a".to_owned(),
            budget_id: Some("budget-a".to_owned()),
            spend: Some(Decimal::from(7_i32)),
        }])
    })
    .await;
    drop(guard);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].token, "token-a");
    assert!(!logs.text().contains("Failed to fetch"));
}

fn assert_every_subject_is_named(text: &str) {
    for subject in [
        "team memberships",
        "keys",
        "orgs",
        "tags",
        "model access groups",
        "projects",
    ] {
        assert!(text.contains(subject), "the warning names {subject}");
    }
}

#[tokio::test]
async fn every_linked_table_warns_under_its_own_subject() {
    let mut rows = FakeRows::empty();
    rows.team_memberships = Err("read failed".to_owned());
    rows.keys = Err("read failed".to_owned());
    rows.organizations = Err("read failed".to_owned());
    rows.tags = Err("read failed".to_owned());
    rows.model_access_groups = Err("read failed".to_owned());
    rows.projects = Err("read failed".to_owned());
    let budgets = vec![budget("budget-a", None, None)];
    let logs = Logs::default();
    let guard = set_default(subscriber(logs.clone()));
    let collected =
        collect_budget_cascade(&rows, &budgets, DISABLED, &RecordingSchedule::new(at()))
            .await
            .expect("collection survives every read failure");
    drop(guard);
    assert!(collected.counter_resets.is_empty());
    assert!(collected.cache_keys.is_empty());
    assert_every_subject_is_named(&logs.text());
}

#[tokio::test]
async fn a_failed_table_keeps_collecting_the_other_tables() {
    let mut rows = FakeRows::empty();
    rows.keys = Err("key table read failed".to_owned());
    rows.organizations = Ok(vec![OrganizationRow {
        organization_id: "org-fetched".to_owned(),
        budget_id: Some("budget-a".to_owned()),
        spend: Some(Decimal::from(4_i32)),
    }]);
    let budgets = vec![budget("budget-a", None, None)];
    let logs = Logs::default();
    let guard = set_default(subscriber(logs.clone()));
    let collected =
        collect_budget_cascade(&rows, &budgets, DISABLED, &RecordingSchedule::new(at()))
            .await
            .expect("collection survives the key failure");
    drop(guard);
    assert!(logs.text().contains("key table read failed"));
    assert_eq!(
        collected.counter_resets,
        vec![CounterReset {
            counter_key: "spend:org:org-fetched".to_owned(),
            carried_spend: Decimal::ZERO,
        }]
    );
    assert_eq!(
        collected.cache_keys,
        vec![
            "org_id:org-fetched".to_owned(),
            "org_id:org-fetched:with_budget".to_owned(),
        ]
    );
}

#[tokio::test]
async fn a_failed_fetch_still_commits_the_reset_by_budget_id() {
    let database = Database::provision().await;
    seed_linked_rows(&database.client).await;
    let mut rows = FakeRows::empty();
    rows.keys = Err("key table read failed".to_owned());
    let budgets = vec![budget("budget-a", None, None)];
    let collected =
        collect_budget_cascade(&rows, &budgets, DISABLED, &RecordingSchedule::new(at()))
            .await
            .expect("collection survives the key failure");
    commit_budget_cascade(&database.store, &collected, None)
        .await
        .expect("cascade commits");
    assert_eq!(
        spend_of(
            &database.client,
            "LiteLLM_VerificationToken",
            "token",
            "token-a"
        )
        .await,
        Decimal::ZERO
    );
    assert_eq!(
        spend_of(
            &database.client,
            "LiteLLM_OrganizationTable",
            "organization_id",
            "org-a"
        )
        .await,
        Decimal::ZERO
    );
    database.finish().await;
}
