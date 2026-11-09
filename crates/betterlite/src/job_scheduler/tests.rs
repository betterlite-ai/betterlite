#![cfg(test)]

use std::collections::BTreeMap;
use std::io::Result as IoResult;
use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, TimeDelta, Utc};
use tokio::task::yield_now;
use tokio::time::Instant;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use betterlite_config::sections::scheduled_job_stagger::Settings;

use crate::scheduled_job_stagger::apply::apply_scheduled_job_stagger;
use crate::scheduled_job_stagger::identity::SystemIdentityEnvironment;
use crate::scheduled_job_stagger::{
    JobScheduler as _, MONTHLY_SPEND_REPORT_JOB_ID, NextRunTime, Trigger,
};

use super::env::JobDefaults;
use super::triggers::{CronTrigger, CronZone, IntervalTrigger};
use super::{BoxFuture, Clock, JobFn, JobSpec, ProxyScheduler, SchedulerState};

pub(crate) fn origin() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_225_600, 0).unwrap()
}

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

pub(crate) fn defaults() -> JobDefaults {
    JobDefaults {
        coalesce: true,
        misfire_grace_seconds: 3600,
        max_instances: 1,
    }
}

pub(crate) struct TokioClock {
    origin: DateTime<Utc>,
    started: Instant,
}

impl TokioClock {
    pub(crate) fn new(origin: DateTime<Utc>) -> Self {
        Self {
            origin,
            started: Instant::now(),
        }
    }
}

impl Clock for TokioClock {
    fn now(&self) -> DateTime<Utc> {
        let elapsed = self.started.elapsed();
        match TimeDelta::from_std(elapsed) {
            Ok(delta) => self.origin.checked_add_signed(delta).unwrap_or(self.origin),
            Err(_source) => self.origin,
        }
    }
}

pub(crate) async fn settle() {
    for _ in 0_usize..16 {
        yield_now().await;
    }
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).unwrap()
    }
}

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogBuffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_max_level(tracing::Level::TRACE)
        .without_time()
        .with_ansi(false)
        .finish();
    let guard = set_default(subscriber);
    (buffer, guard)
}

fn noop_run() -> JobFn {
    Arc::new(|| {
        let future: BoxFuture<'static, ()> = Box::pin(async {});
        future
    })
}

struct FixedTrigger(DateTime<Utc>);

impl Trigger for FixedTrigger {
    fn next_fire_time(
        &self,
        _previous_fire_time: Option<DateTime<Utc>>,
        _now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        Some(self.0)
    }
}

fn interval_spec(id: &str, seconds: u64) -> JobSpec {
    JobSpec {
        id: id.to_owned(),
        trigger: Box::new(IntervalTrigger { seconds }),
        run: noop_run(),
        replace_existing: false,
        next_run_time: NextRunTime::Unset,
        misfire_grace_seconds: None,
        max_instances: None,
    }
}

#[test]
fn add_job_refuses_a_duplicate_id_without_replace_existing() {
    let mut scheduler = ProxyScheduler::new(defaults(), Arc::new(super::SystemClock));
    scheduler.add_job(interval_spec("nightly", 60)).unwrap();
    let error = scheduler.add_job(interval_spec("nightly", 60)).unwrap_err();
    assert!(matches!(error, super::SchedulerError::ConflictingId { .. }));
    let mut replacement = interval_spec("nightly", 120);
    replacement.replace_existing = true;
    scheduler.add_job(replacement).unwrap();
    assert_eq!(
        scheduler.get_job("nightly").unwrap().period_seconds,
        Some(120)
    );
}

#[test]
fn remove_job_reports_an_unknown_id_and_drops_a_known_one() {
    let mut scheduler = ProxyScheduler::new(defaults(), Arc::new(super::SystemClock));
    scheduler.add_job(interval_spec("nightly", 60)).unwrap();
    let error = scheduler.remove_job("missing").unwrap_err();
    assert!(matches!(error, super::SchedulerError::JobLookup { .. }));
    scheduler.remove_job("nightly").unwrap();
    assert!(scheduler.get_job("nightly").is_none());
}

#[test]
fn pause_and_resume_leave_a_stopped_scheduler_stopped() {
    let mut scheduler = ProxyScheduler::new(defaults(), Arc::new(super::SystemClock));
    scheduler.pause();
    assert_eq!(scheduler.state(), SchedulerState::Stopped);
    scheduler.resume();
    assert_eq!(scheduler.state(), SchedulerState::Stopped);
}

#[test]
fn the_job_trait_reports_snapshots_and_replaces_triggers() {
    let start = origin();
    let mut scheduler = ProxyScheduler::new(defaults(), Arc::new(TokioClock::new(start)));
    scheduler.add_job(interval_spec("nightly", 60)).unwrap();
    let snapshots = scheduler.jobs();
    assert_eq!(snapshots.len(), 1);
    let Some(snapshot) = snapshots.first() else {
        panic!("the job snapshot is missing");
    };
    assert_eq!(snapshot.id, "nightly");
    assert_eq!(snapshot.next_run_time, NextRunTime::Unset);
    assert_eq!(snapshot.period_seconds, Some(60));
    scheduler.modify_job("nightly", Box::new(FixedTrigger(start)));
    let view = scheduler.get_job("nightly").unwrap();
    assert_eq!(view.scheduled_for, Some(start));
    assert_eq!(view.next_run_time, NextRunTime::Unset);
    assert_eq!(view.period_seconds, None);
}

#[tokio::test]
async fn running_is_true_once_started_and_after_a_pause() {
    let mut scheduler = ProxyScheduler::new(defaults(), Arc::new(TokioClock::new(origin())));
    assert!(!scheduler.running());
    scheduler.start();
    assert!(scheduler.running());
    scheduler.pause();
    assert_eq!(scheduler.state(), SchedulerState::Paused);
    assert!(scheduler.running());
    scheduler.shutdown();
    assert!(!scheduler.running());
}

#[tokio::test(start_paused = true)]
async fn apply_scheduled_job_stagger_shifts_a_cron_job_through_modify_job() {
    let start = origin();
    let mut scheduler = ProxyScheduler::new(defaults(), Arc::new(TokioClock::new(start)));
    let mut spec = interval_spec(MONTHLY_SPEND_REPORT_JOB_ID, 60);
    spec.trigger = Box::new(CronTrigger::from_crontab("*/5 * * * *", CronZone::Utc).unwrap());
    spec.next_run_time = NextRunTime::Unset;
    scheduler.add_job(spec).unwrap();
    let mut offsets = BTreeMap::new();
    offsets.insert(MONTHLY_SPEND_REPORT_JOB_ID.to_owned(), 120_i64);
    let settings = Settings {
        enabled: true,
        window_seconds: 300,
        identity: None,
        offsets,
    };
    let applied = apply_scheduled_job_stagger(
        &mut scheduler,
        &settings,
        Some("test"),
        &BTreeMap::new(),
        &SystemIdentityEnvironment,
    );
    assert_eq!(applied.get(MONTHLY_SPEND_REPORT_JOB_ID), Some(&120));
    let view = scheduler.get_job(MONTHLY_SPEND_REPORT_JOB_ID).unwrap();
    assert_eq!(view.scheduled_for, Some(start + TimeDelta::seconds(120)));
}

#[tokio::test]
async fn modify_job_keeps_an_explicit_next_run_time() {
    let pinned = instant("2030-01-01T00:00:00Z");
    let mut scheduler = ProxyScheduler::new(defaults(), Arc::new(super::SystemClock));
    let mut spec = interval_spec("anchored_job", 60);
    spec.next_run_time = NextRunTime::Explicit(Some(pinned));
    scheduler.add_job(spec).unwrap();
    scheduler.modify_job("anchored_job", Box::new(FixedTrigger(origin())));
    let view = scheduler.get_job("anchored_job").unwrap();
    assert_eq!(view.scheduled_for, Some(pinned));
    assert_eq!(view.next_run_time, NextRunTime::Explicit(Some(pinned)));
}

#[tokio::test(start_paused = true)]
async fn an_offset_on_an_interval_job_leaves_its_next_fire_unshifted() {
    let start = origin();
    let mut scheduler = ProxyScheduler::new(defaults(), Arc::new(TokioClock::new(start)));
    scheduler
        .add_job(interval_spec("interval_job", 60))
        .unwrap();
    let mut offsets = BTreeMap::new();
    offsets.insert("interval_job".to_owned(), 120_i64);
    let settings = Settings {
        enabled: true,
        window_seconds: 300,
        identity: None,
        offsets,
    };
    let applied = apply_scheduled_job_stagger(
        &mut scheduler,
        &settings,
        Some("test"),
        &BTreeMap::new(),
        &SystemIdentityEnvironment,
    );
    assert_eq!(applied.get("interval_job"), Some(&120));
    let view = scheduler.get_job("interval_job").unwrap();
    assert_eq!(view.scheduled_for, Some(start + TimeDelta::seconds(60)));
}
