use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use sha2::{Digest as _, Sha256};

use crate::spend_backfills::fixtures::{
    DailyBucket, KeySeed, KeyTable, SpendBackfillCase, SpendLogSeed, at,
};
use crate::spend_backfills::{
    BackfillError, BackfillOutcome, BackfillPlan, key_total_spend, key_total_spend_from_logs,
    run_plan,
};

use super::scenarios::{
    ARCHIVE_DISTINCT_SCENARIO, EMPTY_DURATION_SCENARIO, FLOOR_BUDGETS, FLOOR_FAILURE_SCENARIO,
    FLOOR_KEYS, FLOOR_TOTALS, HISTORY_FAILURE_SCENARIO, MIXED_ARCHIVE_SCENARIO,
    NULL_LIFETIME_TOTALS, OWN_LINKED_SCENARIO, SHORT_HISTORY_KEYS, SHORT_HISTORY_LOGS,
    SHORT_HISTORY_TOTALS,
};
use super::{daily_plan, log_seed, money};

const EXISTING_BUCKET_SQL: &str = r#"INSERT INTO "LiteLLM_DailyToolSpend" (date, tool_name, spend, total_tokens, request_count, updated_at) VALUES ('2026-06-10', 'tool-a', 99.0, 999, 42, now())"#;
const NULL_LOGS_SQL: &str = r#"ALTER TABLE "LiteLLM_SpendLogs" ALTER COLUMN spend DROP NOT NULL;
ALTER TABLE "LiteLLM_SpendLogs" ALTER COLUMN total_tokens DROP NOT NULL;"#;
const NULL_SPEND_LOG_SQL: &str = r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, total_tokens, "startTime") VALUES ('null-log', 'sk-test', NULL, NULL, $1)"#;
const NULL_KEY_SPEND_SQL: &str =
    r#"ALTER TABLE "LiteLLM_SpendLogs" ALTER COLUMN spend DROP NOT NULL"#;
const NULL_KEYS_SQL: &str = r#"ALTER TABLE "LiteLLM_VerificationToken" ALTER COLUMN spend DROP NOT NULL, ALTER COLUMN total_spend DROP NOT NULL;
ALTER TABLE "LiteLLM_DeletedVerificationToken" ALTER COLUMN spend DROP NOT NULL, ALTER COLUMN total_spend DROP NOT NULL;"#;
const NULL_LIFETIMES_SQL: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, spend, total_spend) VALUES ('null-spend', NULL, 1.0), ('null-total', 5.0, NULL), ('both-null', NULL, NULL), ('below', 5.0, 1.0), ('equal', 5.0, 5.0), ('higher', 5.0, 10.0);
INSERT INTO "LiteLLM_DeletedVerificationToken" (id, token, spend, total_spend) VALUES ('archived-below', 'token-below', 5.0, 1.0), ('archived-null', 'token-null', NULL, 1.0);"#;
const REJECT_ARCHIVED_SQL: &str = r#"CREATE FUNCTION reject_archived_key_update() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'archived key update rejected'; END $$;
CREATE TRIGGER reject_archived_key_update BEFORE UPDATE ON "LiteLLM_DeletedVerificationToken" FOR EACH ROW EXECUTE FUNCTION reject_archived_key_update();"#;
const ALLOW_ARCHIVED_SQL: &str = r#"DROP TRIGGER reject_archived_key_update ON "LiteLLM_DeletedVerificationToken";
DROP FUNCTION reject_archived_key_update();"#;
const LOG_START: (u32, u32, u32, u32, u32) = (6, 1, 0, 0, 0);

type Amount = (i64, u32);

#[derive(Clone, Copy)]
pub(super) struct BudgetPlan<'plan> {
    pub(super) budget_id: &'plan str,
    pub(super) budget_duration: Option<&'plan str>,
}

#[derive(Clone, Copy)]
pub(super) struct KeyPlan<'plan> {
    pub(super) table: KeyTable,
    pub(super) id: &'plan str,
    pub(super) token: &'plan str,
    pub(super) spend: Amount,
    pub(super) total_spend: Amount,
    pub(super) budget_duration: Option<&'plan str>,
    pub(super) budget_id: Option<&'plan str>,
}

#[derive(Clone, Copy)]
pub(super) struct LogPlan<'plan> {
    pub(super) request_id: &'plan str,
    pub(super) api_key: &'plan str,
    pub(super) spend: Amount,
    pub(super) total_tokens: i32,
}

#[derive(Clone, Copy)]
pub(super) struct ExpectedRow<'plan> {
    pub(super) table: KeyTable,
    pub(super) id: &'plan str,
    pub(super) total_spend: Option<Amount>,
}

pub(super) struct HistoryScenario<'plan> {
    pub(super) budgets: &'plan [BudgetPlan<'plan>],
    pub(super) keys: &'plan [KeyPlan<'plan>],
    pub(super) logs: &'plan [LogPlan<'plan>],
    pub(super) outcome: BackfillOutcome,
    pub(super) totals: &'plan [ExpectedRow<'plan>],
}

pub(super) struct FailureScenario<'plan> {
    pub(super) operation: Operation,
    pub(super) keys: &'plan [KeyPlan<'plan>],
    pub(super) logs: &'plan [LogPlan<'plan>],
}

fn log_time() -> NaiveDateTime {
    at(7, 26, 12, 0, 0)
}

fn bucket(
    date: &str,
    tool_name: &str,
    spend: Decimal,
    total_tokens: i64,
    request_count: i64,
) -> DailyBucket {
    DailyBucket {
        date: date.to_owned(),
        tool_name: tool_name.to_owned(),
        spend,
        total_tokens,
        request_count,
    }
}

fn double_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

async fn daily(case: &SpendBackfillCase) -> BackfillOutcome {
    let session = case.store.session().await.expect("a pooled session");
    run_plan(&**session, daily_plan())
        .await
        .expect("the daily backfill")
}

#[derive(Clone, Copy)]
pub(super) enum Operation {
    Floor,
    History,
}

impl Operation {
    const fn plan(self) -> BackfillPlan {
        match self {
            Self::Floor => BackfillPlan::KeyTotalSpend,
            Self::History => BackfillPlan::KeyTotalSpendFromLogs,
        }
    }

    const fn failure(self) -> &'static str {
        match self {
            Self::Floor => key_total_spend::ARCHIVED_FAILURE,
            Self::History => key_total_spend_from_logs::ARCHIVED_FAILURE,
        }
    }

    const fn committed(self) -> (i64, i64) {
        match self {
            Self::Floor => (5, 2),
            Self::History => (4, 2),
        }
    }

    const fn retried(self) -> (i64, i64) {
        match self {
            Self::Floor => (5, 7),
            Self::History => (4, 6),
        }
    }

    const fn retry_outcome(self) -> BackfillOutcome {
        match self {
            Self::Floor => BackfillOutcome::KeyTotalSpend {
                active_keys: 0,
                archived_keys: 1,
            },
            Self::History => BackfillOutcome::KeyTotalSpendFromLogs {
                active_keys: 0,
                archived_keys: 1,
            },
        }
    }
}

async fn run_operation(
    case: &SpendBackfillCase,
    operation: Operation,
) -> Result<BackfillOutcome, BackfillError> {
    let session = case.store.session().await.expect("a pooled session");
    run_plan(&**session, operation.plan()).await
}

async fn operation_outcome(case: &SpendBackfillCase, operation: Operation) -> BackfillOutcome {
    run_operation(case, operation)
        .await
        .expect("the key spend backfill")
}

async fn seed_budgets(case: &SpendBackfillCase, plans: &[BudgetPlan<'_>]) {
    for plan in plans {
        case.seed_budget(plan.budget_id, plan.budget_duration).await;
    }
}

async fn seed_rows(case: &SpendBackfillCase, keys: &[KeyPlan<'_>], logs: &[LogPlan<'_>]) {
    for plan in keys {
        let (spend_units, spend_scale) = plan.spend;
        let (total_units, total_scale) = plan.total_spend;
        let seed = KeySeed {
            token: plan.token,
            spend: money(spend_units, spend_scale),
            total_spend: money(total_units, total_scale),
            budget_duration: plan.budget_duration,
            budget_id: plan.budget_id,
        };
        case.seed_key(plan.table, plan.id, seed).await;
    }
    for plan in logs {
        let (spend_units, spend_scale) = plan.spend;
        let (month, day, hour, minute, second) = LOG_START;
        let seed = SpendLogSeed {
            request_id: plan.request_id,
            api_key: plan.api_key,
            spend: money(spend_units, spend_scale),
            total_tokens: plan.total_tokens,
            start_time: at(month, day, hour, minute, second),
        };
        case.seed_log(seed).await;
    }
}

async fn assert_buckets(case: &SpendBackfillCase, expected: Vec<DailyBucket>) {
    let actual = case.daily_buckets().await;
    assert_eq!(actual, expected);
}

async fn assert_totals(case: &SpendBackfillCase, expected: &[ExpectedRow<'_>]) {
    for row in expected {
        let actual = case.key_row(row.table, row.id).await.1;
        let total = row.total_spend.map(|(units, scale)| money(units, scale));
        assert_eq!(actual, total);
    }
}

async fn assert_key_total(
    case: &SpendBackfillCase,
    table: KeyTable,
    id: &str,
    total_spend: Option<Decimal>,
) {
    let actual = case.key_row(table, id).await.1;
    assert_eq!(actual, total_spend);
}

async fn run_history_scenario(scenario: HistoryScenario<'_>) {
    let case = SpendBackfillCase::new(None).await;
    seed_budgets(&case, scenario.budgets).await;
    seed_rows(&case, scenario.keys, scenario.logs).await;
    let outcome = operation_outcome(&case, Operation::History).await;
    assert_eq!(outcome, scenario.outcome);
    assert_totals(&case, scenario.totals).await;
    case.close().await;
}

async fn run_failure_scenario(scenario: FailureScenario<'_>) {
    let case = SpendBackfillCase::new(None).await;
    seed_rows(&case, scenario.keys, scenario.logs).await;
    assert_archived_transition(&case, scenario.operation).await;
    case.close().await;
}

async fn assert_archived_transition(case: &SpendBackfillCase, operation: Operation) {
    case.client
        .batch_execute(REJECT_ARCHIVED_SQL)
        .await
        .expect("the rejecting trigger");
    let error = run_operation(case, operation)
        .await
        .expect_err("the archived update fails");
    let BackfillError::Statement { statement, .. } = error;
    assert_eq!(statement, operation.failure());
    let (active_keys, archived_keys) = operation.committed();
    assert_transition_totals(case, active_keys, archived_keys).await;
    case.client
        .batch_execute(ALLOW_ARCHIVED_SQL)
        .await
        .expect("the trigger removal");
    let outcome = operation_outcome(case, operation).await;
    assert_eq!(outcome, operation.retry_outcome());
    let (retried_active, retried_archived) = operation.retried();
    assert_transition_totals(case, retried_active, retried_archived).await;
}

async fn assert_transition_totals(case: &SpendBackfillCase, active: i64, archived: i64) {
    let rows = [
        ExpectedRow {
            table: KeyTable::Active,
            id: "key-active",
            total_spend: Some((active, 0)),
        },
        ExpectedRow {
            table: KeyTable::Archived,
            id: "life-one",
            total_spend: Some((archived, 0)),
        },
    ];
    assert_totals(case, &rows).await;
}

pub(super) async fn cutover_strict_utc() {
    let case = SpendBackfillCase::new(Some("America/New_York")).await;
    case.seed_log(log_seed("before", money(15, 1), 10, log_time()))
        .await;
    case.seed_tool("before", "tool-before", at(7, 24, 23, 59, 59))
        .await;
    case.seed_log(log_seed("exact", money(25, 1), 20, log_time()))
        .await;
    case.seed_tool("exact", "tool-exact", at(7, 25, 0, 0, 0))
        .await;
    case.seed_log(log_seed("after", money(45, 1), 40, log_time()))
        .await;
    case.seed_tool("after", "tool-after", at(7, 25, 0, 0, 1))
        .await;
    let outcome = daily(&case).await;
    assert_eq!(outcome, BackfillOutcome::DailyToolSpend { buckets: 1 });
    assert_buckets(
        &case,
        vec![bucket("2026-07-24", "tool-before", money(15, 1), 10, 1)],
    )
    .await;
    case.close().await;
}

pub(super) async fn index_time_not_log_time() {
    let case = SpendBackfillCase::new(None).await;
    case.seed_log(log_seed("early-index", money(3, 0), 7, at(7, 30, 9, 0, 0)))
        .await;
    case.seed_tool("early-index", "tool-early", at(6, 20, 10, 0, 0))
        .await;
    case.seed_log(log_seed("late-index", money(9, 0), 90, at(6, 1, 9, 0, 0)))
        .await;
    case.seed_tool("late-index", "tool-late", at(7, 30, 10, 0, 0))
        .await;
    let outcome = daily(&case).await;
    assert_eq!(outcome, BackfillOutcome::DailyToolSpend { buckets: 1 });
    assert_buckets(
        &case,
        vec![bucket("2026-06-20", "tool-early", money(3, 0), 7, 1)],
    )
    .await;
    case.close().await;
}

pub(super) async fn null_sums_zero() {
    let case = SpendBackfillCase::new(None).await;
    case.client
        .batch_execute(NULL_LOGS_SQL)
        .await
        .expect("nullable spend columns");
    case.client
        .execute(NULL_SPEND_LOG_SQL, &[&at(6, 1, 0, 0, 0)])
        .await
        .expect("a null spend log");
    case.seed_tool("null-log", "tool-null", at(6, 1, 1, 0, 0))
        .await;
    case.seed_log(log_seed("value-log", money(25, 1), 5, at(6, 2, 0, 0, 0)))
        .await;
    case.seed_tool("value-log", "tool-value", at(6, 2, 1, 0, 0))
        .await;
    let outcome = daily(&case).await;
    assert_eq!(outcome, BackfillOutcome::DailyToolSpend { buckets: 2 });
    assert_buckets(
        &case,
        vec![
            bucket("2026-06-01", "tool-null", Decimal::ZERO, 0, 1),
            bucket("2026-06-02", "tool-value", money(25, 1), 5, 1),
        ],
    )
    .await;
    case.close().await;
}

pub(super) async fn full_spend_per_tool() {
    let case = SpendBackfillCase::new(None).await;
    case.seed_log(log_seed("request-one", money(15, 1), 10, log_time()))
        .await;
    case.seed_tool("request-one", "tool-a", at(6, 10, 12, 0, 0))
        .await;
    case.seed_tool("request-one", "tool-b", at(6, 10, 12, 0, 0))
        .await;
    case.seed_log(log_seed("request-two", money(5, 1), 4, log_time()))
        .await;
    case.seed_tool("request-two", "tool-a", at(6, 11, 12, 0, 0))
        .await;
    case.seed_tool("request-three", "tool-c", at(6, 12, 12, 0, 0))
        .await;
    let outcome = daily(&case).await;
    assert_eq!(outcome, BackfillOutcome::DailyToolSpend { buckets: 3 });
    assert_buckets(
        &case,
        vec![
            bucket("2026-06-10", "tool-a", money(15, 1), 10, 1),
            bucket("2026-06-10", "tool-b", money(15, 1), 10, 1),
            bucket("2026-06-11", "tool-a", money(5, 1), 4, 1),
        ],
    )
    .await;
    case.close().await;
}

pub(super) async fn bucket_conflict_skips_all() {
    let case = SpendBackfillCase::new(None).await;
    case.client
        .execute(EXISTING_BUCKET_SQL, &[])
        .await
        .expect("an existing bucket");
    case.seed_log(log_seed("request-one", money(15, 1), 10, log_time()))
        .await;
    case.seed_tool("request-one", "tool-a", at(6, 10, 12, 0, 0))
        .await;
    case.seed_tool("request-one", "tool-b", at(6, 10, 12, 0, 0))
        .await;
    let outcome = daily(&case).await;
    assert_eq!(outcome, BackfillOutcome::DailyToolSpend { buckets: 1 });
    assert_buckets(
        &case,
        vec![
            bucket("2026-06-10", "tool-a", money(99, 0), 999, 42),
            bucket("2026-06-10", "tool-b", money(15, 1), 10, 1),
        ],
    )
    .await;
    case.close().await;
}

pub(super) async fn floor_no_budget_filter() {
    let case = SpendBackfillCase::new(None).await;
    seed_budgets(&case, &FLOOR_BUDGETS).await;
    seed_rows(&case, &FLOOR_KEYS, &[]).await;
    let outcome = operation_outcome(&case, Operation::Floor).await;
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpend {
            active_keys: 3,
            archived_keys: 2
        }
    );
    assert_totals(&case, &FLOOR_TOTALS).await;
    case.close().await;
}

pub(super) async fn floor_null_unchanged() {
    let case = SpendBackfillCase::new(None).await;
    case.client
        .batch_execute(NULL_KEYS_SQL)
        .await
        .expect("nullable lifetime columns");
    case.client
        .batch_execute(NULL_LIFETIMES_SQL)
        .await
        .expect("the lifetime rows");
    let outcome = operation_outcome(&case, Operation::Floor).await;
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpend {
            active_keys: 1,
            archived_keys: 1
        }
    );
    assert_totals(&case, &NULL_LIFETIME_TOTALS).await;
    case.close().await;
}

pub(super) async fn own_or_linked_reset() {
    run_history_scenario(OWN_LINKED_SCENARIO).await;
}

pub(super) async fn empty_duration_is_reset() {
    run_history_scenario(EMPTY_DURATION_SCENARIO).await;
}

pub(super) async fn token_or_double_hash() {
    let case = SpendBackfillCase::new(None).await;
    let token = "sk-token";
    let digest = double_hash(token);
    let upper = double_hash(token).to_uppercase();
    let key_row = KeySeed {
        token,
        spend: money(0, 0),
        total_spend: money(0, 0),
        budget_duration: Some("1d"),
        budget_id: None,
    };
    case.seed_key(KeyTable::Active, token, key_row).await;
    let logs = [
        ("log-token", token, money(1, 0)),
        ("log-digest", digest.as_str(), money(2, 0)),
        ("log-upper", upper.as_str(), money(100, 0)),
        ("log-other", "other", money(50, 0)),
    ];
    for (request_id, api_key, spend) in logs {
        let log_row = SpendLogSeed {
            request_id,
            api_key,
            spend,
            total_tokens: 0,
            start_time: at(6, 1, 0, 0, 0),
        };
        case.seed_log(log_row).await;
    }
    let outcome = operation_outcome(&case, Operation::History).await;
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpendFromLogs {
            active_keys: 1,
            archived_keys: 0
        }
    );
    assert_key_total(&case, KeyTable::Active, token, Some(money(3, 0))).await;
    case.close().await;
}

pub(super) async fn archive_distinct_tokens() {
    run_history_scenario(ARCHIVE_DISTINCT_SCENARIO).await;
}

pub(super) async fn mixed_archive_lifetimes() {
    run_history_scenario(MIXED_ARCHIVE_SCENARIO).await;
}

pub(super) async fn shorter_history_never_lowers() {
    let case = SpendBackfillCase::new(None).await;
    case.client
        .batch_execute(NULL_KEY_SPEND_SQL)
        .await
        .expect("a nullable spend column");
    seed_rows(&case, &SHORT_HISTORY_KEYS, &SHORT_HISTORY_LOGS).await;
    case.client
        .execute(
            r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, total_tokens, "startTime") VALUES ('log-null', 'key-null-sum', NULL, 0, $1)"#,
            &[&at(6, 1, 0, 0, 0)],
        )
        .await
        .expect("a null spend log");
    let outcome = operation_outcome(&case, Operation::History).await;
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpendFromLogs {
            active_keys: 0,
            archived_keys: 0
        }
    );
    assert_totals(&case, &SHORT_HISTORY_TOTALS).await;
    case.close().await;
}

pub(super) async fn active_commits_before_archive() {
    run_failure_scenario(FLOOR_FAILURE_SCENARIO).await;
}

pub(super) async fn logs_second_write_failure() {
    run_failure_scenario(HISTORY_FAILURE_SCENARIO).await;
}
