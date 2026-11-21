use betterlite_config::sections::settings::{JsonValue, SettingsStore};
use chrono::TimeDelta;

use crate::cleanup_schedule::changes::{CleanupScheduleState, ScheduleChange, on_settings_changed};
use crate::cleanup_schedule::ports::{CleanupScheduler, cleanup_job_spec};
use crate::cleanup_schedule::schedule::{CleanupSchedule, cleanup_schedule_of};
use crate::cleanup_schedule::trigger::ScheduleContext;
use crate::cleanup_schedule::{CLEANUP_CRON_KEY, RETENTION_PERIOD_KEY, SPEND_LOG_CLEANUP_JOB_ID};
use crate::job_scheduler::SchedulerState;
use crate::job_scheduler::triggers::IntervalTrigger;

use super::support::{
    capture_logs, fixture, noop_job, origin, period_of, proxy_scheduler, recording, retained,
    store, with_job,
};

fn apply(
    scheduler: Option<&mut dyn CleanupScheduler>,
    previous: &CleanupSchedule,
    settings: &SettingsStore,
    context: &ScheduleContext<'_>,
    state: &mut CleanupScheduleState,
) {
    let change = ScheduleChange { previous, context };
    on_settings_changed(scheduler, &change, settings, noop_job(), state);
}

#[test]
fn a_stopped_scheduler_skips_the_settings_change_entirely() {
    let fixture = fixture();
    let settings = retained("30d");
    let mut scheduler = recording(SchedulerState::Stopped);
    let mut state = CleanupScheduleState::default();
    apply(
        Some(&mut scheduler),
        &CleanupSchedule::default(),
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert_eq!(scheduler.inserted, 0);
    assert_eq!(state.last_attempt, None);
    assert!(!state.reschedule_failed);
}

#[test]
fn an_absent_scheduler_still_compares_and_records_the_attempt() {
    let fixture = fixture();
    let settings = retained("30d");
    let mut state = CleanupScheduleState::default();
    apply(
        None,
        &CleanupSchedule::default(),
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert_eq!(state.last_attempt, Some(cleanup_schedule_of(&settings)));
    assert!(!state.reschedule_failed);
}

#[test]
fn the_first_settings_change_adds_the_staggered_interval_job() {
    let fixture = fixture();
    let settings = retained("30d");
    let mut scheduler = recording(SchedulerState::Paused);
    let mut state = CleanupScheduleState::default();
    let (buffer, _guard) = capture_logs();
    apply(
        Some(&mut scheduler),
        &CleanupSchedule::default(),
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert_eq!(scheduler.inserted, 1);
    let trigger = scheduler.jobs.get(SPEND_LOG_CLEANUP_JOB_ID).unwrap();
    assert_eq!(
        period_of(trigger.as_ref(), origin()),
        Some(TimeDelta::seconds(2_592_000))
    );
    assert_eq!(state.last_attempt, Some(cleanup_schedule_of(&settings)));
    assert!(
        buffer
            .contents()
            .contains("Spend log cleanup rescheduled with trigger: interval of 1d")
    );
}

#[test]
fn a_cron_settings_change_adds_the_cron_job_and_logs_it() {
    let fixture = fixture();
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (CLEANUP_CRON_KEY, JsonValue::from("* * * * *")),
    ]);
    let mut scheduler = recording(SchedulerState::Paused);
    let mut state = CleanupScheduleState::default();
    let (buffer, _guard) = capture_logs();
    apply(
        Some(&mut scheduler),
        &CleanupSchedule::default(),
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert_eq!(scheduler.inserted, 1);
    assert!(
        buffer
            .contents()
            .contains("Spend log cleanup rescheduled with trigger: cron * * * * *")
    );
}

#[test]
fn an_unchanged_schedule_leaves_the_live_job_alone() {
    let fixture = fixture();
    let settings = retained("30d");
    let previous = cleanup_schedule_of(&settings);
    let mut scheduler = with_job(recording(SchedulerState::Paused));
    let mut state = CleanupScheduleState {
        last_attempt: Some(previous.clone()),
        reschedule_failed: false,
    };
    apply(
        Some(&mut scheduler),
        &previous,
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert_eq!(scheduler.inserted, 0);
    assert!(scheduler.removals.is_empty());
}

#[test]
fn the_baseline_is_the_last_attempt_while_the_job_exists() {
    let fixture = fixture();
    let current = cleanup_schedule_of(&retained("30d"));
    let different_previous = cleanup_schedule_of(&retained("60d"));
    let settings = retained("30d");
    let mut scheduler = with_job(recording(SchedulerState::Paused));
    let mut state = CleanupScheduleState {
        last_attempt: Some(current.clone()),
        reschedule_failed: false,
    };
    apply(
        Some(&mut scheduler),
        &different_previous,
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert_eq!(scheduler.inserted, 0);
}

#[test]
fn a_failed_reschedule_is_retried_on_the_next_sync() {
    let fixture = fixture();
    let settings = retained("30d");
    let mut scheduler = recording(SchedulerState::Paused);
    scheduler.fail_insert = true;
    let mut state = CleanupScheduleState::default();
    let (buffer, _guard) = capture_logs();
    apply(
        Some(&mut scheduler),
        &CleanupSchedule::default(),
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert!(state.reschedule_failed);
    assert_eq!(state.last_attempt, None);
    assert!(
        buffer
            .contents()
            .contains("Spend log cleanup could not be rescheduled, will retry on next sync:")
    );
    scheduler.fail_insert = false;
    apply(
        Some(&mut scheduler),
        &CleanupSchedule::default(),
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert!(!state.reschedule_failed);
    assert_eq!(state.last_attempt, Some(cleanup_schedule_of(&settings)));
    assert_eq!(scheduler.inserted, 2);
}

#[test]
fn a_recorded_attempt_is_not_retried_while_the_schedule_is_unchanged() {
    let fixture = fixture();
    let settings = retained("30d");
    let current = cleanup_schedule_of(&settings);
    let mut scheduler = recording(SchedulerState::Paused);
    let mut state = CleanupScheduleState {
        last_attempt: Some(current.clone()),
        reschedule_failed: true,
    };
    apply(
        Some(&mut scheduler),
        &current,
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert_eq!(scheduler.inserted, 0);
    assert_eq!(state.last_attempt, Some(current));
}

#[test]
fn a_schedule_without_retention_keys_removes_the_live_job() {
    let fixture = fixture();
    let settings = store(&[(CLEANUP_CRON_KEY, JsonValue::from("* * * * *"))]);
    let previous = cleanup_schedule_of(&retained("30d"));
    let mut scheduler = with_job(recording(SchedulerState::Paused));
    let mut state = CleanupScheduleState::default();
    let (buffer, _guard) = capture_logs();
    apply(
        Some(&mut scheduler),
        &previous,
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert_eq!(
        scheduler.removals,
        vec![SPEND_LOG_CLEANUP_JOB_ID.to_owned()]
    );
    assert!(scheduler.jobs.is_empty());
    assert!(
        buffer
            .contents()
            .contains("Removed existing spend log cleanup job")
    );
    assert_eq!(state.last_attempt, Some(cleanup_schedule_of(&settings)));
}

#[test]
fn removing_an_absent_job_is_a_no_op() {
    let fixture = fixture();
    let settings = store(&[]);
    let previous = cleanup_schedule_of(&retained("30d"));
    let mut scheduler = recording(SchedulerState::Paused);
    let mut state = CleanupScheduleState::default();
    apply(
        Some(&mut scheduler),
        &previous,
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert!(scheduler.removals.is_empty());
    assert_eq!(state.last_attempt, Some(CleanupSchedule::default()));
}

#[test]
fn an_unchanged_schedule_without_retention_keys_removes_the_live_job() {
    let fixture = fixture();
    let settings = store(&[]);
    let previous = CleanupSchedule::default();
    let mut scheduler = with_job(recording(SchedulerState::Paused));
    let mut state = CleanupScheduleState::default();
    apply(
        Some(&mut scheduler),
        &previous,
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert_eq!(
        scheduler.removals,
        vec![SPEND_LOG_CLEANUP_JOB_ID.to_owned()]
    );
    assert!(scheduler.jobs.is_empty());
    assert_eq!(state.last_attempt, Some(CleanupSchedule::default()));
}

#[test]
fn a_live_job_without_a_recorded_attempt_and_an_unchanged_schedule_is_left_alone() {
    let fixture = fixture();
    let settings = retained("30d");
    let previous = cleanup_schedule_of(&settings);
    let mut scheduler = with_job(recording(SchedulerState::Paused));
    let mut state = CleanupScheduleState::default();
    apply(
        Some(&mut scheduler),
        &previous,
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert_eq!(scheduler.inserted, 0);
    assert!(scheduler.removals.is_empty());
}

#[test]
fn a_missing_job_is_added_when_the_recorded_attempt_differs_from_the_schedule() {
    let fixture = fixture();
    let settings = retained("30d");
    let current = cleanup_schedule_of(&settings);
    let mut scheduler = recording(SchedulerState::Paused);
    let mut state = CleanupScheduleState {
        last_attempt: Some(cleanup_schedule_of(&retained("60d"))),
        reschedule_failed: false,
    };
    apply(
        Some(&mut scheduler),
        &current,
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert_eq!(scheduler.inserted, 1);
    assert_eq!(state.last_attempt, Some(current));
}

#[tokio::test]
async fn the_proxy_scheduler_port_reports_its_state_and_jobs() {
    let mut scheduler = proxy_scheduler();
    scheduler.start();
    scheduler.pause();
    assert_eq!(scheduler.scheduler_state(), SchedulerState::Paused);
    assert!(!scheduler.has_job(SPEND_LOG_CLEANUP_JOB_ID));
    let spec = cleanup_job_spec(Box::new(IntervalTrigger { seconds: 60 }), noop_job());
    scheduler.insert_job(spec).unwrap();
    assert!(scheduler.has_job(SPEND_LOG_CLEANUP_JOB_ID));
    scheduler
        .remove_existing_job(SPEND_LOG_CLEANUP_JOB_ID)
        .unwrap();
    assert!(!scheduler.has_job(SPEND_LOG_CLEANUP_JOB_ID));
    scheduler.shutdown();
}

#[test]
fn a_recorded_attempt_is_left_alone_when_no_retention_key_remains() {
    let fixture = fixture();
    let settings = store(&[]);
    let previous = CleanupSchedule::default();
    let recorded = cleanup_schedule_of(&retained("30d"));
    let mut scheduler = recording(SchedulerState::Paused);
    let mut state = CleanupScheduleState {
        last_attempt: Some(recorded.clone()),
        reschedule_failed: false,
    };
    apply(
        Some(&mut scheduler),
        &previous,
        &settings,
        &fixture.context(),
        &mut state,
    );
    assert_eq!(scheduler.inserted, 0);
    assert!(scheduler.removals.is_empty());
    assert_eq!(state.last_attempt, Some(recorded));
}

#[test]
fn an_invalid_new_schedule_keeps_the_live_job_and_is_not_retried() {
    let fixture = fixture();
    let previous = cleanup_schedule_of(&retained("30d"));
    let broken = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (CLEANUP_CRON_KEY, JsonValue::from("not a cron")),
    ]);
    let mut scheduler = with_job(recording(SchedulerState::Paused));
    let mut state = CleanupScheduleState {
        last_attempt: Some(previous.clone()),
        reschedule_failed: false,
    };
    let (buffer, _guard) = capture_logs();
    apply(
        Some(&mut scheduler),
        &previous,
        &broken,
        &fixture.context(),
        &mut state,
    );
    assert_eq!(scheduler.inserted, 0);
    assert_eq!(scheduler.jobs.len(), 1);
    assert_eq!(state.last_attempt, Some(cleanup_schedule_of(&broken)));
    assert!(!state.reschedule_failed);
    let first_logs = buffer
        .contents()
        .matches("Invalid maximum_spend_logs_cleanup_cron")
        .count();
    assert_eq!(first_logs, 1);
    apply(
        Some(&mut scheduler),
        &previous,
        &broken,
        &fixture.context(),
        &mut state,
    );
    let second_logs = buffer
        .contents()
        .matches("Invalid maximum_spend_logs_cleanup_cron")
        .count();
    assert_eq!(second_logs, 1);
}
