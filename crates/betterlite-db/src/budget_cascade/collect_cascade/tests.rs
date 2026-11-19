#![cfg(test)]

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::collect_budget_cascade;
use crate::budget_cascade::linked_rows::{
    KeyRow, LinkedRows, ModelAccessGroupRow, OrganizationRow, PostgresLinkedRows, ProjectRow,
    TagRow, TeamMembershipRow,
};
use crate::budget_cascade::tests::{
    Database, FakeRows, RecordingSchedule, budget, seed_linked_rows, unnamed_budget,
};
use crate::budget_cascade::{BudgetCascade, BudgetReset};
use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::key_hash::KeyHash;

fn at() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("timestamp is valid")
}

const ENABLED: RolloverPolicy = RolloverPolicy { enabled: true };
const DISABLED: RolloverPolicy = RolloverPolicy { enabled: false };
const COUNTER_KEYS: [&str; 6] = [
    "spend:team_member:user-a:team-a",
    "spend:key:token-a",
    "spend:org:org-a",
    "spend:tag:tag-a",
    "spend:model_access_group:group-a",
    "spend:project:project-a",
];
const CACHE_KEYS: [&str; 7] = [
    "team-a_user-a",
    "token-a",
    "org_id:org-a",
    "org_id:org-a:with_budget",
    "tag:tag-a",
    "model_access_group:group-a",
    "project_id:project-a",
];

#[tokio::test]
async fn collect_reads_every_linked_table_for_the_due_tiers() {
    let database = Database::provision().await;
    seed_linked_rows(&database.client).await;
    let budgets = vec![budget("budget-a", None, Some("daily"))];
    let schedule = RecordingSchedule::new(at());
    let rows = PostgresLinkedRows {
        store: database.store.clone(),
    };
    let collected = collect_budget_cascade(&rows, &budgets, DISABLED, &schedule)
        .await
        .expect("collection succeeds");
    assert_eq!(collected.budgets, budgets);
    assert_eq!(collected.budget_ids, vec!["budget-a".to_owned()]);
    assert_eq!(
        collected
            .counter_resets
            .iter()
            .map(|reset| reset.counter_key.as_str())
            .collect::<Vec<_>>(),
        COUNTER_KEYS
    );
    assert_eq!(collected.cache_keys, CACHE_KEYS);
    assert_eq!(
        collected.budget_resets,
        vec![BudgetReset {
            budget_id: "budget-a".to_owned(),
            reset_at: at(),
        }]
    );
    assert_eq!(schedule.durations(), vec!["daily".to_owned()]);
    assert!(collected.rollover_caps.is_empty());
    database.finish().await;
}

#[tokio::test]
async fn collect_leaves_out_windowed_keys_and_unspent_gated_rows() {
    let database = Database::provision().await;
    seed_linked_rows(&database.client).await;
    database
        .client
        .batch_execute(
            "INSERT INTO \"LiteLLM_BudgetTable\" (budget_id) VALUES ('budget-b'); INSERT INTO \"LiteLLM_VerificationToken\" (token, budget_id, spend) VALUES ('token-zero', 'budget-a', 0), ('token-b', 'budget-b', 10); INSERT INTO \"LiteLLM_OrganizationTable\" (organization_id, budget_id, spend) VALUES ('org-zero', 'budget-a', 0); INSERT INTO \"LiteLLM_TeamMembership\" (user_id, team_id, budget_id, spend) VALUES ('user-zero', 'team-zero', 'budget-a', 0);",
        )
        .await
        .expect("extra rows seed");
    let budgets = vec![budget("budget-a", None, None)];
    let rows = PostgresLinkedRows {
        store: database.store.clone(),
    };
    let collected =
        collect_budget_cascade(&rows, &budgets, DISABLED, &RecordingSchedule::new(at()))
            .await
            .expect("collection succeeds");
    let counters: Vec<&str> = collected
        .counter_resets
        .iter()
        .map(|reset| reset.counter_key.as_str())
        .collect();
    assert!(counters.contains(&"spend:team_member:user-zero:team-zero"));
    assert!(!counters.contains(&"spend:key:token-window"));
    assert!(!counters.contains(&"spend:key:token-zero"));
    assert!(!counters.contains(&"spend:key:token-b"));
    assert!(!counters.contains(&"spend:org:org-zero"));
    database.finish().await;
}

#[tokio::test]
async fn collect_reports_the_carried_spend_of_every_linked_row() {
    let mut rows = FakeRows::empty();
    rows.team_memberships = Ok(vec![TeamMembershipRow {
        user_id: "user-a".to_owned(),
        team_id: "team-a".to_owned(),
        budget_id: Some("budget-a".to_owned()),
        spend: Some(Decimal::from(30_i32)),
    }]);
    rows.keys = Ok(vec![KeyRow {
        token: KeyHash::from("token-a"),
        budget_id: Some("budget-a".to_owned()),
        spend: Some(Decimal::from(10_i32)),
    }]);
    rows.organizations = Ok(vec![OrganizationRow {
        organization_id: "org-unlinked".to_owned(),
        budget_id: None,
        spend: Some(Decimal::from(30_i32)),
    }]);
    let budgets = vec![budget("budget-a", Some(Decimal::from(20_i32)), None)];
    let collected = collect_budget_cascade(&rows, &budgets, ENABLED, &RecordingSchedule::new(at()))
        .await
        .expect("collection succeeds");
    assert_eq!(
        collected.rollover_caps.get("budget-a").copied(),
        Some(Decimal::from(20_i32))
    );
    assert_eq!(
        collected
            .counter_resets
            .iter()
            .map(|reset| reset.carried_spend)
            .collect::<Vec<_>>(),
        vec![Decimal::from(10_i32), Decimal::ZERO, Decimal::ZERO,]
    );
}

#[tokio::test]
async fn collect_carries_nothing_when_rollover_is_off() {
    let mut rows = FakeRows::empty();
    rows.projects = Ok(vec![ProjectRow {
        project_id: "project-a".to_owned(),
        budget_id: Some("budget-a".to_owned()),
        spend: Some(Decimal::from(30_i32)),
    }]);
    let budgets = vec![budget("budget-a", Some(Decimal::from(20_i32)), None)];
    let collected =
        collect_budget_cascade(&rows, &budgets, DISABLED, &RecordingSchedule::new(at()))
            .await
            .expect("collection succeeds");
    assert!(collected.rollover_caps.is_empty());
    assert_eq!(collected.counter_resets[0].carried_spend, Decimal::ZERO);
}

#[tokio::test]
async fn collect_returns_the_empty_cascade_without_fetching_without_due_tiers() {
    let schedule = RecordingSchedule::new(at());
    let empty = collect_budget_cascade(&UnreachableRows, &[], DISABLED, &schedule)
        .await
        .expect("collection succeeds");
    assert_eq!(empty, BudgetCascade::default());
    let unnamed =
        collect_budget_cascade(&UnreachableRows, &[unnamed_budget()], DISABLED, &schedule)
            .await
            .expect("collection succeeds");
    assert_eq!(unnamed, BudgetCascade::default());
}

#[tokio::test]
async fn collect_reports_a_schedule_failure() {
    let budgets = vec![budget("budget-a", None, Some("daily"))];
    let result =
        collect_budget_cascade(&FakeRows::empty(), &budgets, DISABLED, &FailingSchedule).await;
    assert!(matches!(result, Err(ScheduleError)));
}

#[derive(Debug, thiserror::Error)]
#[error("reset settings unreadable")]
struct ScheduleError;

struct FailingSchedule;

impl ResetSchedule for FailingSchedule {
    type Failure = ScheduleError;

    fn next_reset_at(&self, _budget_duration: &str) -> Result<DateTime<Utc>, ScheduleError> {
        Err(ScheduleError)
    }
}

struct UnreachableRows;

impl LinkedRows for UnreachableRows {
    type Failure = String;

    async fn team_memberships(
        &self,
        _budget_ids: &[String],
    ) -> Result<Vec<TeamMembershipRow>, String> {
        panic!("no fetch without due tiers");
    }

    async fn keys(&self, _budget_ids: &[String]) -> Result<Vec<KeyRow>, String> {
        panic!("no fetch without due tiers");
    }

    async fn organizations(&self, _budget_ids: &[String]) -> Result<Vec<OrganizationRow>, String> {
        panic!("no fetch without due tiers");
    }

    async fn tags(&self, _budget_ids: &[String]) -> Result<Vec<TagRow>, String> {
        panic!("no fetch without due tiers");
    }

    async fn model_access_groups(
        &self,
        _budget_ids: &[String],
    ) -> Result<Vec<ModelAccessGroupRow>, String> {
        panic!("no fetch without due tiers");
    }

    async fn projects(&self, _budget_ids: &[String]) -> Result<Vec<ProjectRow>, String> {
        panic!("no fetch without due tiers");
    }
}
