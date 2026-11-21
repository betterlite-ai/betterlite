use betterlite_config::sections::settings::{JsonValue, SettingsStore};

use crate::cleanup_schedule::changes::CleanupScheduleState;
use crate::cleanup_schedule::error::CleanupScheduleError;
use crate::cleanup_schedule::ports::CleanupScheduler;
use crate::cleanup_schedule::register::{CleanupRegistration, register_cleanup_job};
use crate::cleanup_schedule::schedule::cleanup_schedule_of;
use crate::cleanup_schedule::trigger::ScheduleContext;
use crate::cleanup_schedule::{
    CLEANUP_CRON_KEY, RETENTION_INTERVAL_KEY, RETENTION_PERIOD_KEY, SPEND_LOG_CLEANUP_JOB_ID,
};
use crate::job_scheduler::SchedulerState;

use super::support::{
    FixedRng, capture_logs, fixture, noop_job, proxy_scheduler, recording, retained, store,
};

fn register(
    scheduler: &mut dyn CleanupScheduler,
    settings: &SettingsStore,
    context: &ScheduleContext<'_>,
    sample: u64,
    state: &mut CleanupScheduleState,
) -> Result<(), CleanupScheduleError> {
    let mut rng = FixedRng(sample);
    let mut registration = CleanupRegistration {
        scheduler,
        settings,
        context: *context,
        job: noop_job(),
    };
    register_cleanup_job(&mut registration, &mut rng, state)
}

#[test]
fn registration_does_nothing_without_a_retention_key() {
    let fixture = fixture();
    let mut scheduler = proxy_scheduler();
    let settings = store(&[(CLEANUP_CRON_KEY, JsonValue::from("* * * * *"))]);
    let mut state = CleanupScheduleState::default();
    register(&mut scheduler, &settings, &fixture.context(), 0, &mut state).unwrap();
    assert!(scheduler.get_job(SPEND_LOG_CLEANUP_JOB_ID).is_none());
    assert_eq!(state.last_attempt, None);
}

#[test]
fn startup_interval_carries_the_random_offset_and_records_the_attempt() {
    let fixture = fixture();
    let settings = retained("90d");
    for (sample, offset) in [(0_u64, 0_u32), (u64::MAX, 60_u32)] {
        let mut scheduler = proxy_scheduler();
        let mut state = CleanupScheduleState::default();
        register(
            &mut scheduler,
            &settings,
            &fixture.context(),
            sample,
            &mut state,
        )
        .unwrap();
        let job = scheduler.get_job(SPEND_LOG_CLEANUP_JOB_ID).unwrap();
        assert_eq!(job.period_seconds, Some(7_776_000 + offset));
        assert_eq!(state.last_attempt, Some(cleanup_schedule_of(&settings)));
    }
}

#[test]
fn startup_cron_adds_the_cron_job_and_records_the_attempt() {
    let fixture = fixture();
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (CLEANUP_CRON_KEY, JsonValue::from("* * * * *")),
    ]);
    let (buffer, _guard) = capture_logs();
    let mut scheduler = proxy_scheduler();
    let mut state = CleanupScheduleState::default();
    register(&mut scheduler, &settings, &fixture.context(), 0, &mut state).unwrap();
    let job = scheduler.get_job(SPEND_LOG_CLEANUP_JOB_ID).unwrap();
    assert_eq!(job.period_seconds, None);
    assert_eq!(state.last_attempt, Some(cleanup_schedule_of(&settings)));
    assert!(
        buffer
            .contents()
            .contains("Spend log cleanup scheduled with cron: * * * * *")
    );
}

#[test]
fn startup_records_a_failed_cron_parse_as_an_attempt_without_a_job() {
    let fixture = fixture();
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (CLEANUP_CRON_KEY, JsonValue::from("not a cron")),
    ]);
    let (buffer, _guard) = capture_logs();
    let mut scheduler = proxy_scheduler();
    let mut state = CleanupScheduleState::default();
    register(&mut scheduler, &settings, &fixture.context(), 0, &mut state).unwrap();
    assert!(scheduler.get_job(SPEND_LOG_CLEANUP_JOB_ID).is_none());
    assert_eq!(state.last_attempt, Some(cleanup_schedule_of(&settings)));
    assert!(
        buffer
            .contents()
            .contains("Invalid maximum_spend_logs_cleanup_cron value: not a cron")
    );
}

#[test]
fn startup_records_a_non_string_cron_as_an_attempt_without_a_job() {
    let fixture = fixture();
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (CLEANUP_CRON_KEY, JsonValue::from(7_i64)),
    ]);
    let (buffer, _guard) = capture_logs();
    let mut scheduler = proxy_scheduler();
    let mut state = CleanupScheduleState::default();
    register(&mut scheduler, &settings, &fixture.context(), 0, &mut state).unwrap();
    assert!(scheduler.get_job(SPEND_LOG_CLEANUP_JOB_ID).is_none());
    assert_eq!(state.last_attempt, Some(cleanup_schedule_of(&settings)));
    assert!(
        buffer
            .contents()
            .contains("Invalid maximum_spend_logs_cleanup_cron value: 7")
    );
}

#[test]
fn startup_records_a_failed_interval_parse_as_an_attempt_without_a_job() {
    let fixture = fixture();
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (RETENTION_INTERVAL_KEY, JsonValue::from(86_400_i64)),
    ]);
    let (buffer, _guard) = capture_logs();
    let mut scheduler = proxy_scheduler();
    let mut state = CleanupScheduleState::default();
    register(&mut scheduler, &settings, &fixture.context(), 0, &mut state).unwrap();
    assert!(scheduler.get_job(SPEND_LOG_CLEANUP_JOB_ID).is_none());
    assert_eq!(state.last_attempt, Some(cleanup_schedule_of(&settings)));
    assert!(
        buffer
            .contents()
            .contains("Invalid maximum_spend_logs_retention_interval value: 86400")
    );
}

#[test]
fn a_startup_offset_past_python_datetime_adds_no_job() {
    let fixture = fixture();
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (RETENTION_INTERVAL_KEY, JsonValue::from("251635075169s")),
    ]);
    let mut scheduler = proxy_scheduler();
    let mut state = CleanupScheduleState::default();
    register(&mut scheduler, &settings, &fixture.context(), 0, &mut state).unwrap();
    let job = scheduler.get_job(SPEND_LOG_CLEANUP_JOB_ID).unwrap();
    assert_eq!(job.period_seconds, None);
    let (buffer, _guard) = capture_logs();
    let mut scheduler = proxy_scheduler();
    let mut state = CleanupScheduleState::default();
    register(
        &mut scheduler,
        &settings,
        &fixture.context(),
        u64::MAX,
        &mut state,
    )
    .unwrap();
    assert!(scheduler.get_job(SPEND_LOG_CLEANUP_JOB_ID).is_none());
    assert_eq!(state.last_attempt, Some(cleanup_schedule_of(&settings)));
    assert!(
        buffer
            .contents()
            .contains("Invalid maximum_spend_logs_retention_interval value: \"251635075169s\"")
    );
}

#[test]
fn startup_records_nothing_when_the_scheduler_refuses_the_job() {
    let fixture = fixture();
    let settings = retained("90d");
    let mut scheduler = recording(SchedulerState::Stopped);
    scheduler.fail_insert = true;
    let mut state = CleanupScheduleState::default();
    let error = register(&mut scheduler, &settings, &fixture.context(), 0, &mut state).unwrap_err();
    assert!(matches!(error, CleanupScheduleError::Scheduler(_)));
    assert_eq!(state.last_attempt, None);
}
