#![cfg(test)]

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use rstest::rstest;
use tokio::time::Instant;

use super::SpendLogCleanup;
use super::cutoff::Cutoff;
use super::deadline::{
    MAX_TIMEOUT_MS, MonotonicClock, TokioClock, group_deadline, remaining_budget,
    statement_timeout_ms,
};
use super::settings::{
    CleanupBounds, CleanupGeneralSettings, CleanupSettings, DurationParser, Retention,
};
use super::table::{CleanupTable, delete_statement, remaining_statement};

const DEFAULTS: CleanupBounds = CleanupBounds {
    batch_size: 1000,
    max_batches: 500,
    run_budget: Duration::from_secs(300),
    batch_timeout: Duration::from_secs(30),
    max_consecutive_failures: 3,
    failure_backoff: Duration::from_millis(500),
    remaining_count_cap: 100_000,
};

struct Durations;

impl DurationParser for Durations {
    type Failure = String;

    fn duration_seconds(&self, spec: &str) -> Result<i64, String> {
        match spec {
            "45s" => Ok(45),
            "5m" => Ok(300),
            "0s" | "0d" => Ok(0),
            "3d" => Ok(259_200),
            "7d" => Ok(604_800),
            "1h" => Ok(3_600),
            other => Err(format!("unsupported duration {other}")),
        }
    }
}

struct FixedClock(Instant);

impl MonotonicClock for FixedClock {
    fn now(&self) -> Instant {
        self.0
    }
}

fn settings(json: &str) -> CleanupSettings {
    CleanupSettings {
        general: serde_json::from_str::<CleanupGeneralSettings>(json).unwrap(),
        defaults: DEFAULTS,
    }
}

fn after(start: Instant, millis: u64) -> Instant {
    start.checked_add(Duration::from_millis(millis)).unwrap()
}

#[rstest]
#[case::absent("{}", 1000)]
#[case::explicit_null(r#"{"maximum_spend_logs_cleanup_batch_size": null}"#, 1000)]
#[case::integer(r#"{"maximum_spend_logs_cleanup_batch_size": 12}"#, 12)]
#[case::one(r#"{"maximum_spend_logs_cleanup_batch_size": 1}"#, 1)]
#[case::zero(r#"{"maximum_spend_logs_cleanup_batch_size": 0}"#, 1000)]
#[case::negative(r#"{"maximum_spend_logs_cleanup_batch_size": -3}"#, 1000)]
#[case::text(r#"{"maximum_spend_logs_cleanup_batch_size": " 7 "}"#, 7)]
#[case::signed_text(r#"{"maximum_spend_logs_cleanup_batch_size": "+9"}"#, 9)]
#[case::fraction_text(r#"{"maximum_spend_logs_cleanup_batch_size": "1.5"}"#, 1000)]
#[case::word(r#"{"maximum_spend_logs_cleanup_batch_size": "many"}"#, 1000)]
#[case::true_flag(r#"{"maximum_spend_logs_cleanup_batch_size": true}"#, 1)]
#[case::false_flag(r#"{"maximum_spend_logs_cleanup_batch_size": false}"#, 1000)]
#[case::truncated_float(r#"{"maximum_spend_logs_cleanup_batch_size": 2.9}"#, 2)]
#[case::float_below_one(r#"{"maximum_spend_logs_cleanup_batch_size": 0.5}"#, 1000)]
#[case::float_out_of_range(r#"{"maximum_spend_logs_cleanup_batch_size": 1e300}"#, 1000)]
#[case::list(r#"{"maximum_spend_logs_cleanup_batch_size": [5]}"#, 1000)]
#[case::mapping(r#"{"maximum_spend_logs_cleanup_batch_size": {"size": 5}}"#, 1000)]
fn batch_size_follows_python_int_coercion(#[case] json: &str, #[case] expected: i64) {
    assert_eq!(settings(json).bounds(&Durations).batch_size, expected);
}

#[rstest]
#[case::absent("{}", 500)]
#[case::integer(r#"{"maximum_spend_logs_cleanup_max_batches": 4}"#, 4)]
#[case::zero(r#"{"maximum_spend_logs_cleanup_max_batches": 0}"#, 500)]
fn max_batches_reads_its_own_key(#[case] json: &str, #[case] expected: i64) {
    let bounds = settings(json).bounds(&Durations);
    assert_eq!(bounds.max_batches, expected);
    assert_eq!(bounds.batch_size, DEFAULTS.batch_size);
}

#[rstest]
#[case::absent("{}", 300, 30)]
#[case::explicit_null(r#"{"maximum_spend_logs_cleanup_run_budget": null}"#, 300, 30)]
#[case::budget(r#"{"maximum_spend_logs_cleanup_run_budget": "5m"}"#, 300, 30)]
#[case::short_budget(r#"{"maximum_spend_logs_cleanup_run_budget": "45s"}"#, 45, 30)]
#[case::timeout(r#"{"maximum_spend_logs_cleanup_batch_timeout": "45s"}"#, 300, 45)]
#[case::zero_budget(r#"{"maximum_spend_logs_cleanup_run_budget": "0s"}"#, 300, 30)]
#[case::unknown_unit(r#"{"maximum_spend_logs_cleanup_batch_timeout": "soon"}"#, 300, 30)]
#[case::bare_number(r#"{"maximum_spend_logs_cleanup_run_budget": 45}"#, 300, 30)]
#[case::flag(r#"{"maximum_spend_logs_cleanup_batch_timeout": true}"#, 300, 30)]
#[case::list(r#"{"maximum_spend_logs_cleanup_run_budget": ["45s"]}"#, 300, 30)]
fn duration_overrides_fall_back_unless_positive(
    #[case] json: &str,
    #[case] budget_seconds: u64,
    #[case] timeout_seconds: u64,
) {
    let bounds = settings(json).bounds(&Durations);
    assert_eq!(bounds.run_budget, Duration::from_secs(budget_seconds));
    assert_eq!(bounds.batch_timeout, Duration::from_secs(timeout_seconds));
}

#[test]
fn overrides_leave_the_environment_only_bounds_alone() {
    let json = r#"{"maximum_spend_logs_cleanup_batch_size": 5, "maximum_spend_logs_cleanup_max_batches": 6, "maximum_spend_logs_cleanup_run_budget": "45s", "maximum_spend_logs_cleanup_batch_timeout": "1h"}"#;
    let expected = CleanupBounds {
        batch_size: 5,
        max_batches: 6,
        run_budget: Duration::from_secs(45),
        batch_timeout: Duration::from_secs(3_600),
        ..DEFAULTS
    };
    assert_eq!(settings(json).bounds(&Durations), expected);
    assert_eq!(settings("{}").bounds(&Durations), DEFAULTS);
}

#[rstest]
#[case::absent("{}", None)]
#[case::explicit_null(r#"{"maximum_spend_logs_retention_period": null}"#, None)]
#[case::text(r#"{"maximum_spend_logs_retention_period": "3d"}"#, Some(259_200))]
#[case::integer_days(r#"{"maximum_spend_logs_retention_period": 7}"#, Some(604_800))]
#[case::zero_is_kept(r#"{"maximum_spend_logs_retention_period": "0d"}"#, Some(0))]
#[case::zero_days(r#"{"maximum_spend_logs_retention_period": 0}"#, Some(0))]
#[case::negative_days(r#"{"maximum_spend_logs_retention_period": -3}"#, None)]
#[case::invalid(r#"{"maximum_spend_logs_retention_period": "forever"}"#, None)]
#[case::float(r#"{"maximum_spend_logs_retention_period": 2.5}"#, None)]
#[case::flag(r#"{"maximum_spend_logs_retention_period": true}"#, None)]
#[case::list(r#"{"maximum_spend_logs_retention_period": ["3d"]}"#, None)]
fn retention_accepts_days_and_strings(#[case] json: &str, #[case] expected: Option<i64>) {
    assert_eq!(
        settings(json).retention_seconds(Retention::SpendLogs, &Durations),
        expected
    );
}

#[rstest]
#[case::spend_logs(Retention::SpendLogs, Some(3_600))]
#[case::sessions(Retention::AutorouterSessions, Some(259_200))]
#[case::health_checks(Retention::HealthChecks, Some(604_800))]
#[case::daily_tags(Retention::DailyTagSpend, Some(300))]
fn each_retention_reads_its_own_key(#[case] retention: Retention, #[case] expected: Option<i64>) {
    let json = r#"{"maximum_spend_logs_retention_period": "1h", "maximum_autorouter_session_retention_period": "3d", "maximum_health_check_retention_period": 7, "maximum_daily_tag_spend_retention_period": "5m"}"#;
    assert_eq!(
        settings(json).retention_seconds(retention, &Durations),
        expected
    );
    assert_eq!(
        settings("{}").retention_seconds(retention, &Durations),
        None
    );
}

#[rstest]
#[case::spend_logs(Retention::SpendLogs, "maximum_spend_logs_retention_period")]
#[case::sessions(
    Retention::AutorouterSessions,
    "maximum_autorouter_session_retention_period"
)]
#[case::health_checks(Retention::HealthChecks, "maximum_health_check_retention_period")]
#[case::daily_tags(Retention::DailyTagSpend, "maximum_daily_tag_spend_retention_period")]
fn retention_keys_match_general_settings(#[case] retention: Retention, #[case] key: &str) {
    assert_eq!(retention.key(), key);
}

#[rstest]
#[case::whole_second("2026-01-02T03:04:05Z", "timestamptz", "2026-01-02T03:04:05+00:00")]
#[case::microseconds(
    "2026-01-02T03:04:05.000123Z",
    "timestamptz",
    "2026-01-02T03:04:05.000123+00:00"
)]
#[case::below_a_microsecond(
    "2026-01-02T03:04:05.000000500Z",
    "timestamptz",
    "2026-01-02T03:04:05+00:00"
)]
fn timestamp_cutoffs_cast_to_timestamptz(
    #[case] moment: &str,
    #[case] cast: &str,
    #[case] text: &str,
) {
    let cutoff = Cutoff::Timestamp(moment.parse::<DateTime<Utc>>().unwrap());
    assert_eq!(cutoff.cast(), cast);
    assert_eq!(cutoff.text(), text);
}

#[test]
fn day_cutoffs_cast_to_text() {
    let cutoff = Cutoff::Day(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap());
    assert_eq!(cutoff.cast(), "text");
    assert_eq!(cutoff.text(), "2026-01-02");
}

#[rstest]
#[case::batch_timeout_is_smaller(10_000, 2_000, 2_000)]
#[case::remaining_is_smaller(1_500, 30_000, 1_500)]
#[case::equal(2_000, 2_000, 2_000)]
#[case::deadline_reached(0, 30_000, 1)]
#[case::zero_batch_timeout(10_000, 0, 1)]
#[case::one_millisecond(1, 30_000, 1)]
#[case::two_milliseconds(2, 30_000, 2)]
#[case::above_postgres_range(4_000_000_000, 4_000_000_000, 2_147_483_647)]
fn statement_timeout_takes_the_smaller_bound(
    #[case] remaining_ms: u64,
    #[case] batch_timeout_ms: u64,
    #[case] expected: u64,
) {
    let now = Instant::now();
    let deadline = after(now, remaining_ms);
    let timeout = Duration::from_millis(batch_timeout_ms);
    assert_eq!(statement_timeout_ms(now, deadline, timeout), expected);
}

#[test]
fn statement_timeout_floors_a_passed_deadline_at_one_millisecond() {
    let deadline = Instant::now();
    let now = after(deadline, 5_000);
    assert_eq!(
        statement_timeout_ms(now, deadline, Duration::from_secs(30)),
        1
    );
    assert_eq!(MAX_TIMEOUT_MS, 2_147_483_647);
}

#[rstest]
#[case::single_group(1, 9_000)]
#[case::two_groups(2, 4_500)]
#[case::three_groups(3, 3_000)]
#[case::no_groups(0, 9_000)]
fn group_deadline_shares_what_is_left(#[case] groups: u32, #[case] share_ms: u64) {
    let start = Instant::now();
    let now = after(start, 1_000);
    let overall = after(start, 10_000);
    assert_eq!(group_deadline(overall, now, groups), after(now, share_ms));
}

#[rstest]
#[case::at_the_deadline(10_000)]
#[case::past_the_deadline(12_000)]
fn group_deadline_keeps_a_spent_deadline(#[case] now_ms: u64) {
    let start = Instant::now();
    let overall = after(start, 10_000);
    assert_eq!(group_deadline(overall, after(start, now_ms), 4), overall);
}

#[test]
fn remaining_budget_stops_at_the_deadline() {
    let start = Instant::now();
    let deadline = after(start, 1_000);
    let early = FixedClock(after(start, 400));
    let mut before = remaining_budget(&early, deadline, Duration::from_secs(30));
    assert_eq!(before(), Some(600));
    let capped = FixedClock(start);
    let mut short = remaining_budget(&capped, deadline, Duration::from_millis(250));
    assert_eq!(short(), Some(250));
    let spent = FixedClock(deadline);
    let mut at = remaining_budget(&spent, deadline, Duration::from_secs(30));
    assert_eq!(at(), None);
}

#[test]
fn tokio_clock_moves_forward() {
    let first = TokioClock.now();
    assert!(TokioClock.now() >= first);
}

#[rstest]
#[case::spend_logs(CleanupTable::SpendLogs, "LiteLLM_SpendLogs", &["request_id", "startTime"], "startTime")]
#[case::tool_index(CleanupTable::ToolIndex, "LiteLLM_SpendLogToolIndex", &["request_id", "tool_name"], "start_time")]
#[case::sessions(CleanupTable::AutoRouterSessions, "LiteLLM_AutoRouterSession", &["api_key", "session_id", "router_name"], "last_turn_at")]
#[case::user_sessions(CleanupTable::AutoRouterUserSessions, "LiteLLM_AutoRouterUserSession", &["user_id", "api_key", "session_id", "router_name"], "last_turn_at")]
#[case::health_checks(CleanupTable::HealthChecks, "LiteLLM_HealthCheckTable", &["health_check_id"], "checked_at")]
#[case::daily_tags(CleanupTable::DailyTagSpend, "LiteLLM_DailyTagSpend", &["id"], "date")]
fn tables_name_their_keys_and_time_column(
    #[case] table: CleanupTable,
    #[case] name: &str,
    #[case] keys: &[&str],
    #[case] time_column: &str,
) {
    assert_eq!(table.name(), name);
    assert_eq!(table.key_columns(), keys);
    assert_eq!(table.time_column(), time_column);
}

#[test]
fn statements_bind_the_cutoff_and_the_limit() {
    let moment = Cutoff::Timestamp(DateTime::<Utc>::UNIX_EPOCH);
    assert_eq!(
        delete_statement(CleanupTable::SpendLogs, moment),
        r#"DELETE FROM "LiteLLM_SpendLogs" WHERE ("request_id", "startTime") IN (SELECT "request_id", "startTime" FROM "LiteLLM_SpendLogs" WHERE "startTime" < $1::timestamptz LIMIT $2)"#
    );
    let day = Cutoff::Day(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap());
    assert_eq!(
        delete_statement(CleanupTable::DailyTagSpend, day),
        r#"DELETE FROM "LiteLLM_DailyTagSpend" WHERE ("id") IN (SELECT "id" FROM "LiteLLM_DailyTagSpend" WHERE "date" < $1::text LIMIT $2)"#
    );
    assert_eq!(
        remaining_statement(CleanupTable::HealthChecks, moment),
        r#"SELECT count(*)::int AS remaining FROM (SELECT 1 FROM "LiteLLM_HealthCheckTable" WHERE "checked_at" < $1::timestamptz LIMIT $2) capped"#
    );
    assert_eq!(
        remaining_statement(CleanupTable::DailyTagSpend, day),
        r#"SELECT count(*)::int AS remaining FROM (SELECT 1 FROM "LiteLLM_DailyTagSpend" WHERE "date" < $1::text LIMIT $2) capped"#
    );
}

#[test]
fn the_constructor_keeps_every_collaborator() {
    let snapshot = settings(r#"{"maximum_spend_logs_cleanup_batch_size": 5}"#);
    let cleanup = SpendLogCleanup::new(
        snapshot.clone(),
        TokioClock,
        Durations,
        "partitions",
        Some("lock"),
    );
    assert_eq!(cleanup.settings, snapshot);
    assert_eq!(cleanup.partitions, "partitions");
    assert_eq!(cleanup.pod_lock, Some("lock"));
    assert_eq!(cleanup.settings.bounds(&cleanup.durations).batch_size, 5);
}
