#![cfg(test)]

use std::sync::atomic::AtomicU32;
use std::time::Duration;

use betterlite_prometheus::collector_registry::CollectorRegistry;
use chrono::NaiveDate;
use rstest::rstest;
use tokio::time::Instant;

use crate::cleanup_metrics::CleanupMetrics;
use crate::spend_partitions::SpendPartitionOperation;
use crate::spend_partitions::calendar::{PartitionClock, PartitionInterval};
use crate::spend_partitions::manager::SpendPartitionManager;
use crate::spend_retention::batches::settings::{CleanupSettings, SettingValue};
use crate::spend_retention::batches::table::CleanupTable;
use crate::spend_retention::batches::{SpendLogCleanup, StopReason};
use crate::spend_retention::runner::progress::RunProgress;
use crate::spend_retention::runner::support::{
    FakeLock, FakePartitions, FixedCleanupClock, PARTITION_NAMES, SESSION_IDS, SPEND_IDS,
    StepCleanupClock, TAG_IDS, TOOL_IDS, TestDurations, USER_SESSION_IDS, group_run, install_logs,
    moment, naive, provision, retention_settings, seed_daily_tag, seed_health_check, seed_session,
    seed_sessions, seed_spend_logs, seed_tool_index, seed_tool_indexes, seed_user_session,
    seed_user_sessions, text_join,
};

use super::{GroupPlan, GroupWindow};

const DAY_PARTITIONS: &str = r#"CREATE TABLE "LiteLLM_SpendLogs_20241201" PARTITION OF "LiteLLM_SpendLogs" FOR VALUES FROM ('2024-12-01') TO ('2024-12-02');
CREATE TABLE "LiteLLM_SpendLogs_straddle" PARTITION OF "LiteLLM_SpendLogs" FOR VALUES FROM ('2024-12-30') TO ('2025-01-06');
CREATE TABLE "LiteLLM_SpendLogs_20250128" PARTITION OF "LiteLLM_SpendLogs" FOR VALUES FROM ('2025-01-28') TO ('2025-01-29');"#;

struct FixedPartitionClock(NaiveDate);

impl PartitionClock for FixedPartitionClock {
    fn today(&self) -> NaiveDate {
        self.0
    }
}

fn fixed_clock() -> FixedCleanupClock {
    FixedCleanupClock {
        utc: moment(2025, 2, 1),
        mono: Instant::now(),
    }
}

fn group_cleanup(
    settings: CleanupSettings,
    partitions: FakePartitions,
) -> SpendLogCleanup<FixedCleanupClock, TestDurations, FakePartitions, FakeLock> {
    SpendLogCleanup::new(settings, fixed_clock(), TestDurations, partitions, None)
}

fn step_cleanup(
    settings: CleanupSettings,
    clock: StepCleanupClock,
) -> SpendLogCleanup<StepCleanupClock, TestDurations, FakePartitions, FakeLock> {
    SpendLogCleanup::new(
        settings,
        clock,
        TestDurations,
        FakePartitions::default(),
        None,
    )
}

fn partition_cleanup(
    settings: CleanupSettings,
) -> SpendLogCleanup<
    FixedCleanupClock,
    TestDurations,
    SpendPartitionManager<FixedPartitionClock>,
    FakeLock,
> {
    let today = NaiveDate::from_ymd_opt(2025, 2, 1).expect("today");
    let partitions = SpendPartitionManager {
        interval: PartitionInterval::Day,
        precreate_ahead: 0,
        clock: FixedPartitionClock(today),
    };
    SpendLogCleanup::new(settings, fixed_clock(), TestDurations, partitions, None)
}

fn deadline_after(start: Instant, seconds: u64) -> Instant {
    start
        .checked_add(Duration::from_secs(seconds))
        .expect("a group deadline")
}

#[tokio::test]
async fn spend_group_deletes_expired_logs_and_tool_index_rows() {
    let mut case = provision().await;
    case.seed_log("old-log", naive(2024, 12, 1, 0)).await;
    case.seed_log("fresh-log", naive(2025, 1, 28, 0)).await;
    seed_tool_index(&case, "old-tool", moment(2024, 12, 1)).await;
    seed_tool_index(&case, "fresh-tool", moment(2025, 1, 28)).await;
    let settings = retention_settings(r#"{"maximum_spend_logs_retention_period": "30d"}"#);
    let cleanup = group_cleanup(settings, FakePartitions::default());
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let progress = RunProgress::default();
    let mono = cleanup.clock.mono;
    let mut run = group_run(&mut case.client, &cleanup.clock, &metrics, &progress, mono);
    let deadline = deadline_after(mono, 300);
    let window = GroupWindow {
        retention_seconds: 2_592_000,
        deadline,
    };
    let results = cleanup
        .clean_group(&mut run, GroupPlan::SpendLogs, window)
        .await
        .expect("the spend group");
    assert_eq!(results[0].table, CleanupTable::SpendLogs);
    assert_eq!(results[0].rows_deleted, 1);
    assert_eq!(results[1].table, CleanupTable::ToolIndex);
    assert_eq!(results[1].rows_deleted, 1);
    assert_eq!(text_join(&case.client, SPEND_IDS).await, "fresh-log");
    assert_eq!(text_join(&case.client, TOOL_IDS).await, "fresh-tool");
    case.close().await;
}

#[tokio::test]
async fn session_group_prunes_both_rollups_by_their_last_turn() {
    let mut case = provision().await;
    seed_session(&case, "old-session", moment(2024, 12, 1)).await;
    seed_session(&case, "fresh-session", moment(2025, 1, 28)).await;
    seed_user_session(&case, "old-user-session", moment(2024, 12, 1)).await;
    seed_user_session(&case, "fresh-user-session", moment(2025, 1, 28)).await;
    let settings = retention_settings(r#"{"maximum_autorouter_session_retention_period": "30d"}"#);
    let cleanup = group_cleanup(settings, FakePartitions::default());
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let progress = RunProgress::default();
    let mono = cleanup.clock.mono;
    let mut run = group_run(&mut case.client, &cleanup.clock, &metrics, &progress, mono);
    let deadline = deadline_after(mono, 300);
    let window = GroupWindow {
        retention_seconds: 2_592_000,
        deadline,
    };
    let results = cleanup
        .clean_group(&mut run, GroupPlan::Sessions, window)
        .await
        .expect("the session group");
    assert_eq!(results[0].table, CleanupTable::AutoRouterSessions);
    assert_eq!(results[0].rows_deleted, 1);
    assert_eq!(results[1].table, CleanupTable::AutoRouterUserSessions);
    assert_eq!(results[1].rows_deleted, 1);
    assert_eq!(text_join(&case.client, SESSION_IDS).await, "fresh-session");
    assert_eq!(
        text_join(&case.client, USER_SESSION_IDS).await,
        "fresh-user-session"
    );
    case.close().await;
}

#[tokio::test]
async fn health_group_prunes_checks_by_their_own_retention() {
    let mut case = provision().await;
    seed_health_check(&case, "old-check", moment(2024, 12, 1)).await;
    seed_health_check(&case, "fresh-check", moment(2025, 1, 31)).await;
    let settings = retention_settings(r#"{"maximum_health_check_retention_period": "1d"}"#);
    let cleanup = group_cleanup(settings, FakePartitions::default());
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let progress = RunProgress::default();
    let mono = cleanup.clock.mono;
    let mut run = group_run(&mut case.client, &cleanup.clock, &metrics, &progress, mono);
    let deadline = deadline_after(mono, 300);
    let window = GroupWindow {
        retention_seconds: 86_400,
        deadline,
    };
    let results = cleanup
        .clean_group(&mut run, GroupPlan::HealthChecks, window)
        .await
        .expect("the health-check group");
    assert_eq!(results[0].table, CleanupTable::HealthChecks);
    assert_eq!(results[0].rows_deleted, 1);
    assert_eq!(text_join(&case.client, HEALTH_IDS).await, "fresh-check");
    case.close().await;
}

#[tokio::test]
async fn daily_tag_group_keeps_the_horizon_day_and_non_iso_keys() {
    let mut case = provision().await;
    seed_daily_tag(&case, "old-day", "2024-12-01").await;
    seed_daily_tag(&case, "before-horizon", "2025-01-01").await;
    seed_daily_tag(&case, "horizon", "2025-01-02").await;
    seed_daily_tag(&case, "lifetime", "Lifetime").await;
    let settings = retention_settings(r#"{"maximum_daily_tag_spend_retention_period": "30d"}"#);
    let cleanup = group_cleanup(settings, FakePartitions::default());
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let progress = RunProgress::default();
    let mono = cleanup.clock.mono;
    let mut run = group_run(&mut case.client, &cleanup.clock, &metrics, &progress, mono);
    let deadline = deadline_after(mono, 300);
    let window = GroupWindow {
        retention_seconds: 2_592_000,
        deadline,
    };
    let results = cleanup
        .clean_group(&mut run, GroupPlan::DailyTags, window)
        .await
        .expect("the daily tag group");
    assert_eq!(results[0].table, CleanupTable::DailyTagSpend);
    assert_eq!(results[0].rows_deleted, 2);
    assert_eq!(text_join(&case.client, TAG_IDS).await, "horizon,lifetime");
    case.close().await;
}

#[tokio::test]
async fn expired_partitions_drop_before_their_stragglers_are_deleted() {
    let mut case = provision().await;
    case.run_ok(SpendPartitionOperation::Partition).await;
    case.client
        .batch_execute(DAY_PARTITIONS)
        .await
        .expect("the day partitions");
    case.seed_log("dropped-row", naive(2024, 12, 1, 0)).await;
    case.seed_log("straddler-old", naive(2024, 12, 31, 0)).await;
    case.seed_log("straddler-fresh", naive(2025, 1, 5, 0)).await;
    case.seed_log("default-old", naive(2024, 12, 15, 0)).await;
    case.seed_log("default-fresh", naive(2025, 1, 20, 0)).await;
    case.seed_log("fresh-row", naive(2025, 1, 28, 0)).await;
    let settings = retention_settings(
        r#"{"maximum_spend_logs_retention_period": "30d", "use_spend_logs_partitioning": true}"#,
    );
    let cleanup = partition_cleanup(settings);
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let progress = RunProgress::default();
    let mono = cleanup.clock.mono;
    let mut run = group_run(&mut case.client, &cleanup.clock, &metrics, &progress, mono);
    let deadline = deadline_after(mono, 300);
    let window = GroupWindow {
        retention_seconds: 2_592_000,
        deadline,
    };
    let results = cleanup
        .clean_group(&mut run, GroupPlan::SpendLogs, window)
        .await
        .expect("the spend group");
    assert_eq!(results[0].rows_deleted, 2);
    assert_eq!(
        text_join(&case.client, SPEND_IDS).await,
        "default-fresh,fresh-row,straddler-fresh"
    );
    let partitions = text_join(&case.client, PARTITION_NAMES).await;
    assert!(!partitions.contains("LiteLLM_SpendLogs_20241201"));
    assert!(partitions.contains("LiteLLM_SpendLogs_straddle"));
    assert!(partitions.contains("LiteLLM_SpendLogs_p20250201"));
    case.close().await;
}

#[tokio::test]
async fn a_switch_on_table_that_is_not_partitioned_skips_partition_ddl() {
    let mut case = provision().await;
    case.seed_log("old-log", naive(2024, 12, 1, 0)).await;
    let settings = retention_settings(
        r#"{"maximum_spend_logs_retention_period": "30d", "use_spend_logs_partitioning": true}"#,
    );
    let cleanup = partition_cleanup(settings);
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let progress = RunProgress::default();
    let mono = cleanup.clock.mono;
    let mut run = group_run(&mut case.client, &cleanup.clock, &metrics, &progress, mono);
    let deadline = deadline_after(mono, 300);
    let window = GroupWindow {
        retention_seconds: 2_592_000,
        deadline,
    };
    let (logs, _subscriber) = install_logs();
    let results = cleanup
        .clean_group(&mut run, GroupPlan::SpendLogs, window)
        .await
        .expect("the spend group");
    assert_eq!(results[0].rows_deleted, 1);
    assert!(
        !logs
            .contents()
            .contains("Failed to ensure spend-log partition")
    );
    case.close().await;
}

#[rstest]
#[case::switch_off(None, true, &[], &[])]
#[case::failed_probe(Some(true), false, &["probe"], &[300_000])]
#[case::partitioned_table(Some(true), true, &["probe", "ensure", "drop"], &[300_000])]
#[tokio::test]
async fn partition_maintenance_needs_the_switch_and_a_successful_probe(
    #[case] switch: Option<bool>,
    #[case] partitioned: bool,
    #[case] expected: &[&str],
    #[case] budget: &[u64],
) {
    let mut case = provision().await;
    let mut settings = retention_settings(r#"{"maximum_spend_logs_retention_period": "30d"}"#);
    settings.general.use_spend_logs_partitioning = switch.map(SettingValue::Flag);
    let partitions = if partitioned {
        FakePartitions::enabled(false)
    } else {
        FakePartitions::default()
    };
    let cleanup = group_cleanup(settings, partitions);
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let progress = RunProgress::default();
    let mono = cleanup.clock.mono;
    let mut run = group_run(&mut case.client, &cleanup.clock, &metrics, &progress, mono);
    let deadline = deadline_after(mono, 300);
    let window = GroupWindow {
        retention_seconds: 2_592_000,
        deadline,
    };
    cleanup
        .clean_group(&mut run, GroupPlan::SpendLogs, window)
        .await
        .expect("the spend group");
    assert_eq!(cleanup.partitions.calls(), expected);
    assert_eq!(cleanup.partitions.seen_remaining(), budget);
    case.close().await;
}

#[tokio::test]
async fn a_spent_budget_skips_partition_maintenance() {
    let mut case = provision().await;
    let settings = retention_settings(
        r#"{"maximum_spend_logs_retention_period": "30d", "use_spend_logs_partitioning": true}"#,
    );
    let start = Instant::now();
    let clock = StepCleanupClock {
        utc: moment(2025, 2, 1),
        start,
        step: Duration::from_secs(1_000),
        readings: AtomicU32::new(0),
    };
    let cleanup = step_cleanup(settings, clock);
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let progress = RunProgress::default();
    let mut run = group_run(&mut case.client, &cleanup.clock, &metrics, &progress, start);
    let window = GroupWindow {
        retention_seconds: 2_592_000,
        deadline: start,
    };
    let results = cleanup
        .clean_group(&mut run, GroupPlan::SpendLogs, window)
        .await
        .expect("the spend group");
    assert_eq!(results[0].stop_reason, StopReason::BudgetExhausted);
    assert_eq!(results[1].stop_reason, StopReason::BudgetExhausted);
    assert!(cleanup.partitions.calls().is_empty());
    case.close().await;
}

#[tokio::test]
async fn the_logs_table_gets_half_the_group_time_and_the_index_the_rest() {
    let mut case = provision().await;
    seed_spend_logs(&case, naive(2025, 1, 1, 0), 20).await;
    seed_tool_indexes(&case, moment(2024, 12, 1), 20).await;
    let settings = retention_settings(r#"{"maximum_spend_logs_retention_period": "30d"}"#);
    let start = Instant::now();
    let clock = StepCleanupClock {
        utc: moment(2025, 2, 1),
        start,
        step: Duration::from_secs(5),
        readings: AtomicU32::new(0),
    };
    let cleanup = step_cleanup(settings, clock);
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let progress = RunProgress::default();
    let mut run = group_run(&mut case.client, &cleanup.clock, &metrics, &progress, start);
    let deadline = deadline_after(start, 100);
    let window = GroupWindow {
        retention_seconds: 2_592_000,
        deadline,
    };
    let results = cleanup
        .clean_group(&mut run, GroupPlan::SpendLogs, window)
        .await
        .expect("the spend group");
    assert_eq!(results[0].rows_deleted, 6);
    assert_eq!(results[0].stop_reason, StopReason::BudgetExhausted);
    assert_eq!(results[1].rows_deleted, 6);
    case.close().await;
}

#[tokio::test]
async fn the_session_rollups_split_their_group_time() {
    let mut case = provision().await;
    seed_sessions(&case, moment(2024, 12, 1), 20).await;
    seed_user_sessions(&case, moment(2024, 12, 1), 20).await;
    let settings = retention_settings(r#"{"maximum_autorouter_session_retention_period": "30d"}"#);
    let start = Instant::now();
    let clock = StepCleanupClock {
        utc: moment(2025, 2, 1),
        start,
        step: Duration::from_secs(5),
        readings: AtomicU32::new(0),
    };
    let cleanup = step_cleanup(settings, clock);
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let progress = RunProgress::default();
    let mut run = group_run(&mut case.client, &cleanup.clock, &metrics, &progress, start);
    let deadline = deadline_after(start, 100);
    let window = GroupWindow {
        retention_seconds: 2_592_000,
        deadline,
    };
    let results = cleanup
        .clean_group(&mut run, GroupPlan::Sessions, window)
        .await
        .expect("the session group");
    assert_eq!(results[0].rows_deleted, 6);
    assert_eq!(results[0].stop_reason, StopReason::BudgetExhausted);
    assert_eq!(results[1].rows_deleted, 6);
    assert_eq!(results[1].stop_reason, StopReason::BudgetExhausted);
    case.close().await;
}
