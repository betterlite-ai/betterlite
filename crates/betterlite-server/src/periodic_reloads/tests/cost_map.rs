use chrono::{DateTime, Utc};
use rstest::rstest;

use super::support::{
    Harness, capture_logs, general_settings, moment, moment_at, reloaded, run_cost_map_check,
    schedule, unavailable,
};

#[tokio::test]
async fn an_absent_schedule_does_nothing() {
    let harness = Harness::new(moment(1));
    let settings = general_settings(&["model_cost_map"]);
    run_cost_map_check(&harness, &settings).await;
    assert_eq!(harness.cost_map.calls(), 0);
    assert!(harness.rows.recorded().is_empty());
    assert_eq!(harness.checks.reloads.cost_map_applied_revision(), 0);
}

#[tokio::test]
async fn a_schedule_that_is_not_due_does_nothing() {
    let harness = Harness::new(moment(1));
    harness
        .rows
        .set_schedule(schedule(0, Some(2), Some(moment(1))));
    let settings = general_settings(&["model_cost_map"]);
    run_cost_map_check(&harness, &settings).await;
    assert_eq!(harness.cost_map.calls(), 0);
    assert!(harness.rows.recorded().is_empty());
    assert_eq!(harness.checks.reloads.cost_map_loaded_at(), moment(1));
}

#[rstest]
#[case::just_before_the_interval(moment_at(2, 59), false)]
#[case::exactly_at_the_interval(moment(3), true)]
#[tokio::test]
async fn an_interval_schedule_measures_from_the_pod_loaded_at(
    #[case] now: DateTime<Utc>,
    #[case] due: bool,
) {
    let harness = Harness::new(moment(1));
    // the stored last run is in the future, so only the pod's loaded_at can make this due
    harness
        .rows
        .set_schedule(schedule(0, Some(2), Some(moment(10))));
    harness.clock.set(now);
    harness.cost_map.queue(reloaded());
    let settings = general_settings(&["model_cost_map"]);
    run_cost_map_check(&harness, &settings).await;
    assert_eq!(harness.cost_map.calls(), usize::from(due));
    let expected_loaded_at = if due { now } else { moment(1) };
    assert_eq!(
        harness.checks.reloads.cost_map_loaded_at(),
        expected_loaded_at
    );
}

#[tokio::test]
async fn an_unavailable_fetch_keeps_the_current_pricing() {
    let (logs, guard) = capture_logs();
    let harness = Harness::new(moment(1));
    harness.rows.set_schedule(schedule(1, None, None));
    harness.cost_map.queue(unavailable("the fetch timed out"));
    let settings = general_settings(&["model_cost_map"]);
    run_cost_map_check(&harness, &settings).await;
    drop(guard);
    assert_eq!(harness.cost_map.calls(), 1);
    assert!(harness.rows.recorded().is_empty());
    assert_eq!(harness.checks.reloads.cost_map_applied_revision(), 0);
    assert_eq!(harness.checks.reloads.cost_map_loaded_at(), moment(1));
    assert!(logs.contents().contains(
        "Model cost map reload failed (the fetch timed out); keeping current pricing data. The revision stays unapplied so this pod retries on its next poll"
    ));
}

#[tokio::test]
async fn a_reloaded_snapshot_is_adopted_recorded_and_logged() {
    let (logs, guard) = capture_logs();
    let harness = Harness::new(moment(1));
    harness.clock.set(moment(4));
    harness.rows.set_schedule(schedule(3, None, None));
    harness.cost_map.queue(reloaded());
    let settings = general_settings(&["model_cost_map"]);
    run_cost_map_check(&harness, &settings).await;
    drop(guard);
    assert_eq!(harness.rows.recorded(), vec![moment(4)]);
    assert_eq!(harness.checks.reloads.cost_map_applied_revision(), 3);
    assert_eq!(harness.checks.reloads.cost_map_loaded_at(), moment(4));
    assert!(
        logs.contents()
            .contains("Model cost map reloaded successfully. Models count: 2")
    );
}

#[tokio::test]
async fn a_failed_record_leaves_the_revision_unapplied() {
    let (logs, guard) = capture_logs();
    let harness = Harness::new(moment(1));
    harness.clock.set(moment(4));
    harness.rows.set_schedule(schedule(3, None, None));
    harness.rows.fail("record_run");
    harness.cost_map.queue(reloaded());
    let settings = general_settings(&["model_cost_map"]);
    run_cost_map_check(&harness, &settings).await;
    drop(guard);
    // the snapshot is swapped in before the record, as LiteLLM's adopt order does
    assert_eq!(harness.checks.reloads.cost_map_loaded_at(), moment(4));
    assert_eq!(harness.checks.reloads.cost_map_applied_revision(), 0);
    assert!(logs.contents().contains(
        "Error in _check_and_reload_model_cost_map: the model cost map reload run could not be recorded: the fake config repository refused the record_run call"
    ));
}

#[tokio::test]
async fn a_schedule_read_failure_is_logged_and_swallowed() {
    let (logs, guard) = capture_logs();
    let harness = Harness::new(moment(1));
    harness.rows.fail("read_schedule");
    let settings = general_settings(&["model_cost_map"]);
    run_cost_map_check(&harness, &settings).await;
    drop(guard);
    assert_eq!(harness.cost_map.calls(), 0);
    assert!(logs.contents().contains(
        "Error in _check_and_reload_model_cost_map: the model cost map reload schedule could not be read: the fake config repository refused the read_schedule call"
    ));
}
