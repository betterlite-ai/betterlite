use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use sha2::{Digest as _, Sha256};

use crate::spend_backfills::fixtures::{DailyBucket, SpendBackfillCase, SpendLogSeed, at};
use crate::spend_backfills::{
    BackfillError, BackfillOutcome, BackfillPlan, key_total_spend, key_total_spend_from_logs,
    run_plan,
};

use super::{daily_plan, key, log_seed, money};

const EXISTING_BUCKET_SQL: &str = r#"INSERT INTO "LiteLLM_DailyToolSpend" (date, tool_name, spend, total_tokens, request_count, updated_at) VALUES ('2026-06-10', 'tool-a', 99.0, 999, 42, now())"#;
const NULL_LOGS_SQL: &str = r#"ALTER TABLE "LiteLLM_SpendLogs" ALTER COLUMN spend DROP NOT NULL;
ALTER TABLE "LiteLLM_SpendLogs" ALTER COLUMN total_tokens DROP NOT NULL;"#;
const NULL_SPEND_LOG_SQL: &str = r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, total_tokens, "startTime") VALUES ('null-log', 'sk-test', NULL, NULL, $1)"#;
const NULL_KEY_SPEND_SQL: &str =
    r#"ALTER TABLE "LiteLLM_SpendLogs" ALTER COLUMN spend DROP NOT NULL"#;
const NULL_KEYS_SQL: &str = r#"ALTER TABLE "LiteLLM_VerificationToken" ALTER COLUMN spend DROP NOT NULL, ALTER COLUMN total_spend DROP NOT NULL;
ALTER TABLE "LiteLLM_DeletedVerificationToken" ALTER COLUMN spend DROP NOT NULL, ALTER COLUMN total_spend DROP NOT NULL;"#;
const NULL_LIFETIMES_SQL: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, spend, total_spend) VALUES ('null-spend', NULL, 1.0), ('null-total', 5.0, NULL), ('both-null', NULL, NULL), ('below', 5.0, 1.0), ('equal', 5.0, 5.0), ('higher', 10.0, 5.0);
INSERT INTO "LiteLLM_DeletedVerificationToken" (id, token, spend, total_spend) VALUES ('archived-below', 'token-below', 5.0, 1.0), ('archived-null', 'token-null', NULL, 1.0);"#;
const REJECT_ARCHIVED_SQL: &str = r#"CREATE FUNCTION reject_archived_key_update() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'archived key update rejected'; END $$;
CREATE TRIGGER reject_archived_key_update BEFORE UPDATE ON "LiteLLM_DeletedVerificationToken" FOR EACH ROW EXECUTE FUNCTION reject_archived_key_update();"#;
const ALLOW_ARCHIVED_SQL: &str = r#"DROP TRIGGER reject_archived_key_update ON "LiteLLM_DeletedVerificationToken";
DROP FUNCTION reject_archived_key_update();"#;

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
    run_plan(&session, daily_plan())
        .await
        .expect("the daily backfill")
}

async fn floor(case: &SpendBackfillCase) -> BackfillOutcome {
    let session = case.store.session().await.expect("a pooled session");
    run_plan(&session, BackfillPlan::KeyTotalSpend)
        .await
        .expect("the floor backfill")
}

async fn history(case: &SpendBackfillCase) -> BackfillOutcome {
    let session = case.store.session().await.expect("a pooled session");
    run_plan(&session, BackfillPlan::KeyTotalSpendFromLogs)
        .await
        .expect("the history backfill")
}

async fn floor_error(case: &SpendBackfillCase) -> BackfillError {
    let session = case.store.session().await.expect("a pooled session");
    run_plan(&session, BackfillPlan::KeyTotalSpend)
        .await
        .expect_err("the archived update fails")
}

async fn history_error(case: &SpendBackfillCase) -> BackfillError {
    let session = case.store.session().await.expect("a pooled session");
    run_plan(&session, BackfillPlan::KeyTotalSpendFromLogs)
        .await
        .expect_err("the archived update fails")
}

async fn assert_buckets(case: &SpendBackfillCase, expected: Vec<DailyBucket>) {
    let actual = case.daily_buckets().await;
    assert_eq!(actual, expected);
}

async fn assert_active_row(
    case: &SpendBackfillCase,
    token: &str,
    spend: Option<Decimal>,
    total_spend: Option<Decimal>,
) {
    let actual = case.active_row(token).await;
    assert_eq!(actual, (spend, total_spend));
}

async fn assert_archived_row(
    case: &SpendBackfillCase,
    id: &str,
    spend: Option<Decimal>,
    total_spend: Option<Decimal>,
) {
    let actual = case.archived_row(id).await;
    assert_eq!(actual, (spend, total_spend));
}

async fn assert_active_total(case: &SpendBackfillCase, token: &str, total_spend: Option<Decimal>) {
    let actual = case.active_row(token).await.1;
    assert_eq!(actual, total_spend);
}

async fn assert_archived_total(case: &SpendBackfillCase, id: &str, total_spend: Option<Decimal>) {
    let actual = case.archived_row(id).await.1;
    assert_eq!(actual, total_spend);
}

async fn seed_key_log(case: &SpendBackfillCase, request_id: &str, api_key: &str, spend: Decimal) {
    case.seed_log(SpendLogSeed {
        request_id,
        api_key,
        spend,
        total_tokens: 0,
        start_time: at(6, 1, 0, 0, 0),
    })
    .await;
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
    seed_floor_keys(&case).await;
    let outcome = floor(&case).await;
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpend {
            active_keys: 3,
            archived_keys: 2
        }
    );
    assert_active_total(&case, "key-own", Some(money(5, 0))).await;
    assert_active_total(&case, "key-linked", Some(money(5, 0))).await;
    assert_active_total(&case, "key-plain", Some(money(5, 0))).await;
    assert_archived_total(&case, "archived-own", Some(money(7, 0))).await;
    assert_archived_total(&case, "archived-plain", Some(money(7, 0))).await;
    case.close().await;
}

async fn seed_floor_keys(case: &SpendBackfillCase) {
    case.seed_budget("budget-linked", Some("30d")).await;
    case.seed_active_key(key("key-own", money(5, 0), money(1, 0), Some("1d"), None))
        .await;
    case.seed_active_key(key(
        "key-linked",
        money(5, 0),
        money(1, 0),
        None,
        Some("budget-linked"),
    ))
    .await;
    case.seed_active_key(key("key-plain", money(5, 0), money(1, 0), None, None))
        .await;
    case.seed_archived_key(
        "archived-own",
        key(
            "archived-own-token",
            money(7, 0),
            money(2, 0),
            Some("1d"),
            None,
        ),
    )
    .await;
    case.seed_archived_key(
        "archived-plain",
        key("archived-plain-token", money(7, 0), money(2, 0), None, None),
    )
    .await;
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
    let outcome = floor(&case).await;
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpend {
            active_keys: 1,
            archived_keys: 1
        }
    );
    assert_active_row(&case, "null-spend", None, Some(money(1, 0))).await;
    assert_active_row(&case, "null-total", Some(money(5, 0)), None).await;
    assert_active_row(&case, "both-null", None, None).await;
    assert_active_row(&case, "below", Some(money(5, 0)), Some(money(5, 0))).await;
    assert_active_row(&case, "equal", Some(money(5, 0)), Some(money(5, 0))).await;
    assert_active_row(&case, "higher", Some(money(10, 0)), Some(money(5, 0))).await;
    assert_archived_row(
        &case,
        "archived-below",
        Some(money(5, 0)),
        Some(money(5, 0)),
    )
    .await;
    assert_archived_row(&case, "archived-null", None, Some(money(1, 0))).await;
    case.close().await;
}

pub(super) async fn own_or_linked_reset() {
    let case = SpendBackfillCase::new(None).await;
    case.seed_budget("budget-linked", Some("30d")).await;
    case.seed_budget("budget-plain", None).await;
    case.seed_active_key(key("key-own", money(0, 0), money(0, 0), Some("1d"), None))
        .await;
    case.seed_active_key(key(
        "key-linked",
        money(0, 0),
        money(0, 0),
        None,
        Some("budget-linked"),
    ))
    .await;
    case.seed_active_key(key(
        "key-no-reset",
        money(0, 0),
        money(0, 0),
        None,
        Some("budget-plain"),
    ))
    .await;
    seed_reset_logs(&case).await;
    let outcome = history(&case).await;
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpendFromLogs {
            active_keys: 2,
            archived_keys: 0
        }
    );
    assert_active_total(&case, "key-own", Some(money(3, 0))).await;
    assert_active_total(&case, "key-linked", Some(money(4, 0))).await;
    assert_active_total(&case, "key-no-reset", Some(money(0, 0))).await;
    case.close().await;
}

async fn seed_reset_logs(case: &SpendBackfillCase) {
    seed_key_log(case, "log-own", "key-own", money(3, 0)).await;
    seed_key_log(case, "log-linked", "key-linked", money(4, 0)).await;
    seed_key_log(case, "log-no-reset", "key-no-reset", money(9, 0)).await;
}

pub(super) async fn empty_duration_is_reset() {
    let case = SpendBackfillCase::new(None).await;
    case.seed_budget("budget-empty", Some("")).await;
    case.seed_budget("budget-null", None).await;
    case.seed_active_key(key("key-empty", money(0, 0), money(0, 0), Some(""), None))
        .await;
    case.seed_active_key(key(
        "key-empty-linked",
        money(0, 0),
        money(0, 0),
        None,
        Some("budget-empty"),
    ))
    .await;
    case.seed_active_key(key(
        "key-null-linked",
        money(0, 0),
        money(0, 0),
        None,
        Some("budget-null"),
    ))
    .await;
    seed_empty_duration_logs(&case).await;
    let outcome = history(&case).await;
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpendFromLogs {
            active_keys: 2,
            archived_keys: 0
        }
    );
    assert_active_total(&case, "key-empty", Some(money(25, 1))).await;
    assert_active_total(&case, "key-empty-linked", Some(money(15, 1))).await;
    assert_active_total(&case, "key-null-linked", Some(money(0, 0))).await;
    case.close().await;
}

async fn seed_empty_duration_logs(case: &SpendBackfillCase) {
    seed_key_log(case, "log-empty", "key-empty", money(25, 1)).await;
    seed_key_log(case, "log-empty-linked", "key-empty-linked", money(15, 1)).await;
    seed_key_log(case, "log-null-linked", "key-null-linked", money(7, 0)).await;
}

pub(super) async fn token_or_double_hash() {
    let case = SpendBackfillCase::new(None).await;
    let token = "sk-token";
    let digest = double_hash(token);
    let upper = double_hash(token).to_uppercase();
    case.seed_active_key(key(token, money(0, 0), money(0, 0), Some("1d"), None))
        .await;
    seed_key_log(&case, "log-token", token, money(1, 0)).await;
    seed_key_log(&case, "log-digest", digest.as_str(), money(2, 0)).await;
    seed_key_log(&case, "log-upper", upper.as_str(), money(100, 0)).await;
    seed_key_log(&case, "log-other", "other", money(50, 0)).await;
    let outcome = history(&case).await;
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpendFromLogs {
            active_keys: 1,
            archived_keys: 0
        }
    );
    assert_active_total(&case, token, Some(money(3, 0))).await;
    case.close().await;
}

pub(super) async fn archive_distinct_tokens() {
    let case = SpendBackfillCase::new(None).await;
    case.seed_archived_key(
        "life-one",
        key("archived-token", money(1, 0), money(0, 0), Some("1d"), None),
    )
    .await;
    case.seed_archived_key(
        "life-two",
        key("archived-token", money(2, 0), money(0, 0), Some("1d"), None),
    )
    .await;
    case.seed_archived_key(
        "life-plain",
        key("archived-token", money(0, 0), money(0, 0), None, None),
    )
    .await;
    seed_key_log(&case, "log-token", "archived-token", money(3, 0)).await;
    let outcome = history(&case).await;
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpendFromLogs {
            active_keys: 0,
            archived_keys: 2
        }
    );
    assert_archived_total(&case, "life-one", Some(money(3, 0))).await;
    assert_archived_total(&case, "life-two", Some(money(3, 0))).await;
    assert_archived_total(&case, "life-plain", Some(money(0, 0))).await;
    case.close().await;
}

pub(super) async fn mixed_archive_lifetimes() {
    let case = SpendBackfillCase::new(None).await;
    seed_mixed_lifetimes(&case).await;
    let outcome = history(&case).await;
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpendFromLogs {
            active_keys: 0,
            archived_keys: 2
        }
    );
    assert_archived_total(&case, "own-reset", Some(money(2, 0))).await;
    assert_archived_total(&case, "own-plain", Some(money(0, 0))).await;
    assert_archived_total(&case, "linked-reset", Some(money(3, 0))).await;
    assert_archived_total(&case, "linked-plain", Some(money(0, 0))).await;
    case.close().await;
}

async fn seed_mixed_lifetimes(case: &SpendBackfillCase) {
    case.seed_budget("budget-reset", Some("1d")).await;
    case.seed_archived_key(
        "own-reset",
        key("mixed-token", money(0, 0), money(0, 0), Some("1d"), None),
    )
    .await;
    case.seed_archived_key(
        "own-plain",
        key("mixed-token", money(0, 0), money(0, 0), None, None),
    )
    .await;
    case.seed_archived_key(
        "linked-reset",
        key(
            "linked-token",
            money(0, 0),
            money(0, 0),
            None,
            Some("budget-reset"),
        ),
    )
    .await;
    case.seed_archived_key(
        "linked-plain",
        key("linked-token", money(0, 0), money(0, 0), None, None),
    )
    .await;
    seed_key_log(case, "log-mixed", "mixed-token", money(2, 0)).await;
    seed_key_log(case, "log-linked", "linked-token", money(3, 0)).await;
}

pub(super) async fn shorter_history_never_lowers() {
    let case = SpendBackfillCase::new(None).await;
    case.client
        .batch_execute(NULL_KEY_SPEND_SQL)
        .await
        .expect("a nullable spend column");
    seed_short_history_keys(&case).await;
    seed_short_history_logs(&case).await;
    let outcome = history(&case).await;
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpendFromLogs {
            active_keys: 0,
            archived_keys: 0
        }
    );
    assert_active_total(&case, "key-short", Some(money(5, 0))).await;
    assert_active_total(&case, "key-missing", Some(money(2, 0))).await;
    assert_active_total(&case, "key-null-sum", Some(money(1, 0))).await;
    assert_active_total(&case, "key-plain", Some(money(0, 0))).await;
    case.close().await;
}

async fn seed_short_history_keys(case: &SpendBackfillCase) {
    case.seed_active_key(key("key-short", money(5, 0), money(5, 0), Some("1d"), None))
        .await;
    case.seed_active_key(key(
        "key-missing",
        money(2, 0),
        money(2, 0),
        Some("1d"),
        None,
    ))
    .await;
    case.seed_active_key(key(
        "key-null-sum",
        money(1, 0),
        money(1, 0),
        Some("1d"),
        None,
    ))
    .await;
    case.seed_active_key(key("key-plain", money(9, 0), money(0, 0), None, None))
        .await;
}

async fn seed_short_history_logs(case: &SpendBackfillCase) {
    seed_key_log(case, "log-short", "key-short", money(2, 0)).await;
    seed_key_log(case, "log-plain", "key-plain", money(9, 0)).await;
    case.client
        .execute(
            r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, total_tokens, "startTime") VALUES ('log-null', 'key-null-sum', NULL, 0, $1)"#,
            &[&at(6, 1, 0, 0, 0)],
        )
        .await
        .expect("a null spend log");
}

pub(super) async fn active_commits_before_archive() {
    let case = SpendBackfillCase::new(None).await;
    case.seed_active_key(key("key-active", money(5, 0), money(1, 0), None, None))
        .await;
    case.seed_archived_key(
        "life-one",
        key("token-one", money(7, 0), money(2, 0), None, None),
    )
    .await;
    assert_floor_archived_failure(&case).await;
    assert_floor_retry(&case).await;
    case.close().await;
}

async fn assert_floor_archived_failure(case: &SpendBackfillCase) {
    case.client
        .batch_execute(REJECT_ARCHIVED_SQL)
        .await
        .expect("the rejecting trigger");
    let error = floor_error(case).await;
    let BackfillError::Statement { statement, .. } = error;
    assert_eq!(statement, key_total_spend::ARCHIVED_FAILURE);
    assert_active_total(case, "key-active", Some(money(5, 0))).await;
    assert_archived_total(case, "life-one", Some(money(2, 0))).await;
}

async fn assert_floor_retry(case: &SpendBackfillCase) {
    case.client
        .batch_execute(ALLOW_ARCHIVED_SQL)
        .await
        .expect("the trigger removal");
    let outcome = floor(case).await;
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpend {
            active_keys: 0,
            archived_keys: 1
        }
    );
    assert_active_total(case, "key-active", Some(money(5, 0))).await;
    assert_archived_total(case, "life-one", Some(money(7, 0))).await;
}

pub(super) async fn logs_second_write_failure() {
    let case = SpendBackfillCase::new(None).await;
    case.seed_active_key(key(
        "key-active",
        money(1, 0),
        money(1, 0),
        Some("1d"),
        None,
    ))
    .await;
    case.seed_archived_key(
        "life-one",
        key("token-one", money(2, 0), money(2, 0), Some("1d"), None),
    )
    .await;
    seed_key_log(&case, "log-active", "key-active", money(4, 0)).await;
    seed_key_log(&case, "log-archived", "token-one", money(6, 0)).await;
    assert_history_archived_failure(&case).await;
    assert_history_retry(&case).await;
    case.close().await;
}

async fn assert_history_archived_failure(case: &SpendBackfillCase) {
    case.client
        .batch_execute(REJECT_ARCHIVED_SQL)
        .await
        .expect("the rejecting trigger");
    let error = history_error(case).await;
    let BackfillError::Statement { statement, .. } = error;
    assert_eq!(statement, key_total_spend_from_logs::ARCHIVED_FAILURE);
    assert_active_total(case, "key-active", Some(money(4, 0))).await;
    assert_archived_total(case, "life-one", Some(money(2, 0))).await;
}

async fn assert_history_retry(case: &SpendBackfillCase) {
    case.client
        .batch_execute(ALLOW_ARCHIVED_SQL)
        .await
        .expect("the trigger removal");
    let outcome = history(case).await;
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpendFromLogs {
            active_keys: 0,
            archived_keys: 1
        }
    );
    assert_active_total(case, "key-active", Some(money(4, 0))).await;
    assert_archived_total(case, "life-one", Some(money(6, 0))).await;
}
