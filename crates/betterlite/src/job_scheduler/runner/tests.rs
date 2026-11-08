#![cfg(test)]

use std::future::pending;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use tokio::time::advance;

use crate::job_scheduler::env::JobDefaults;
use crate::job_scheduler::tests::{TokioClock, capture_logs, defaults, origin, settle};
use crate::job_scheduler::triggers::IntervalTrigger;
use crate::job_scheduler::{BoxFuture, JobFn, JobSpec, ProxyScheduler, SchedulerState};
use crate::scheduled_job_stagger::{JobScheduler as _, JobSubmission, NextRunTime, Trigger};

type SeenRuns = Vec<(String, Vec<DateTime<Utc>>)>;

fn counting_run(counter: &Arc<AtomicUsize>) -> JobFn {
    let shared = Arc::clone(counter);
    Arc::new(move || {
        let run_counter = Arc::clone(&shared);
        let future: BoxFuture<'static, ()> = Box::pin(async move {
            run_counter.fetch_add(1, Ordering::SeqCst);
        });
        future
    })
}

fn never_finishes() -> BoxFuture<'static, ()> {
    Box::pin(pending())
}

fn hanging_run(counter: &Arc<AtomicUsize>) -> JobFn {
    let shared = Arc::clone(counter);
    Arc::new(move || {
        shared.fetch_add(1, Ordering::SeqCst);
        never_finishes()
    })
}

fn interval_spec(id: &str, seconds: u64, run: JobFn) -> JobSpec {
    JobSpec {
        id: id.to_owned(),
        trigger: Box::new(IntervalTrigger { seconds }),
        run,
        replace_existing: false,
        next_run_time: NextRunTime::Unset,
        misfire_grace_seconds: None,
        max_instances: None,
    }
}

fn scheduler_with(defaults: JobDefaults) -> ProxyScheduler {
    ProxyScheduler::new(defaults, Arc::new(TokioClock::new(origin())))
}

#[tokio::test(start_paused = true)]
async fn a_due_interval_job_runs_when_its_time_arrives() {
    let mut scheduler = scheduler_with(defaults());
    let runs = Arc::new(AtomicUsize::new(0));
    scheduler
        .add_job(interval_spec("interval_job", 60, counting_run(&runs)))
        .unwrap();
    scheduler.start();
    advance(Duration::from_secs(59)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    advance(Duration::from_secs(1)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    scheduler.shutdown();
}

#[tokio::test(start_paused = true)]
async fn a_run_past_its_misfire_grace_is_skipped_with_the_warning() {
    let mut scheduler = scheduler_with(JobDefaults {
        coalesce: true,
        misfire_grace_seconds: 0,
        max_instances: 1,
    });
    let runs = Arc::new(AtomicUsize::new(0));
    let mut spec = interval_spec("late_job", 600, counting_run(&runs));
    spec.next_run_time = NextRunTime::Explicit(Some(origin()));
    scheduler.add_job(spec).unwrap();
    advance(Duration::from_secs(10)).await;
    let (logs, _guard) = capture_logs();
    scheduler.start();
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    assert!(
        logs.contents()
            .contains("Run time of job late_job was missed by")
    );
    advance(Duration::from_secs(590)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    scheduler.shutdown();
}

#[tokio::test(start_paused = true)]
async fn coalesced_missed_runs_fire_once() {
    let mut scheduler = scheduler_with(defaults());
    let runs = Arc::new(AtomicUsize::new(0));
    let seen_times: Arc<Mutex<Vec<DateTime<Utc>>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen_times);
    scheduler.add_submission_listener(Box::new(move |event: &dyn JobSubmission| {
        sink.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(event.scheduled_run_times().iter().copied());
    }));
    scheduler
        .add_job(interval_spec("coalesced_job", 10, counting_run(&runs)))
        .unwrap();
    advance(Duration::from_secs(55)).await;
    scheduler.start();
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    let times = seen_times.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(times.len(), 1);
    assert_eq!(
        times.first().copied(),
        Some(origin() + TimeDelta::seconds(50))
    );
    drop(times);
    scheduler.shutdown();
}

#[tokio::test(start_paused = true)]
async fn every_missed_run_fires_without_coalescing() {
    let mut scheduler = scheduler_with(JobDefaults {
        coalesce: false,
        misfire_grace_seconds: 3600,
        max_instances: 1,
    });
    let runs = Arc::new(AtomicUsize::new(0));
    scheduler
        .add_job(interval_spec("separate_job", 10, counting_run(&runs)))
        .unwrap();
    advance(Duration::from_secs(55)).await;
    scheduler.start();
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 5);
    scheduler.shutdown();
}

#[tokio::test(start_paused = true)]
async fn a_second_instance_is_skipped_with_the_warning() {
    let mut scheduler = scheduler_with(defaults());
    let runs = Arc::new(AtomicUsize::new(0));
    scheduler
        .add_job(interval_spec("slow_job", 10, hanging_run(&runs)))
        .unwrap();
    let (logs, _guard) = capture_logs();
    scheduler.start();
    advance(Duration::from_secs(10)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    advance(Duration::from_secs(10)).await;
    settle().await;
    assert!(logs.contents().contains(
        "Execution of job slow_job skipped: maximum number of running instances reached (1)"
    ));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    scheduler.shutdown();
}

#[tokio::test(start_paused = true)]
async fn a_panicking_job_is_logged_and_the_scheduler_keeps_going() {
    let mut scheduler = scheduler_with(defaults());
    let runs = Arc::new(AtomicUsize::new(0));
    let shared = Arc::clone(&runs);
    let run: JobFn = Arc::new(move || {
        let previous = shared.fetch_add(1, Ordering::SeqCst);
        assert!(previous != 0, "the job threw");
        let future: BoxFuture<'static, ()> = Box::pin(async {});
        future
    });
    scheduler
        .add_job(interval_spec("panicking_job", 10, run))
        .unwrap();
    let (logs, _guard) = capture_logs();
    scheduler.start();
    advance(Duration::from_secs(10)).await;
    settle().await;
    advance(Duration::from_secs(10)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    assert!(logs.contents().contains("scheduled job panicked"));
    assert_eq!(scheduler.state(), SchedulerState::Running);
    scheduler.shutdown();
}

#[tokio::test(start_paused = true)]
async fn pause_stops_dispatch_and_resume_starts_it_again() {
    let mut scheduler = scheduler_with(defaults());
    let runs = Arc::new(AtomicUsize::new(0));
    scheduler
        .add_job(interval_spec("paused_job", 10, counting_run(&runs)))
        .unwrap();
    scheduler.start();
    advance(Duration::from_secs(10)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    scheduler.pause();
    assert!(scheduler.running());
    advance(Duration::from_secs(50)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    scheduler.resume();
    advance(Duration::from_secs(10)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    scheduler.shutdown();
}

#[tokio::test(start_paused = true)]
async fn shutdown_stops_dispatch_and_clears_in_flight_jobs() {
    let mut scheduler = scheduler_with(defaults());
    let runs = Arc::new(AtomicUsize::new(0));
    scheduler
        .add_job(interval_spec("hanging_job", 10, hanging_run(&runs)))
        .unwrap();
    scheduler.start();
    advance(Duration::from_secs(10)).await;
    settle().await;
    assert_eq!(scheduler.in_flight().len(), 1);
    scheduler.shutdown();
    settle().await;
    assert_eq!(scheduler.in_flight().len(), 0);
    assert_eq!(scheduler.state(), SchedulerState::Stopped);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn a_submission_listener_sees_every_run_with_its_scheduled_time() {
    let mut scheduler = scheduler_with(defaults());
    let runs = Arc::new(AtomicUsize::new(0));
    let seen: Arc<Mutex<SeenRuns>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    scheduler.add_submission_listener(Box::new(move |event: &dyn JobSubmission| {
        sink.lock().unwrap_or_else(PoisonError::into_inner).push((
            event.job_id().to_owned(),
            event.scheduled_run_times().to_vec(),
        ));
    }));
    scheduler
        .add_job(interval_spec("observed_job", 10, counting_run(&runs)))
        .unwrap();
    scheduler.start();
    advance(Duration::from_secs(10)).await;
    settle().await;
    let events = seen.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(events.len(), 1);
    let Some((job_id, times)) = events.first() else {
        panic!("the submission listener saw no run");
    };
    assert_eq!(job_id, "observed_job");
    assert_eq!(times.len(), 1);
    assert_eq!(
        times.first().copied(),
        Some(origin() + TimeDelta::seconds(10))
    );
    drop(events);
    scheduler.shutdown();
}

#[tokio::test(start_paused = true)]
async fn an_explicitly_paused_job_never_runs() {
    let mut scheduler = scheduler_with(defaults());
    let runs = Arc::new(AtomicUsize::new(0));
    let mut spec = interval_spec("sleeping_job", 10, counting_run(&runs));
    spec.next_run_time = NextRunTime::Explicit(None);
    scheduler.add_job(spec).unwrap();
    scheduler.start();
    advance(Duration::from_secs(60)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    scheduler.shutdown();
}

struct Once(DateTime<Utc>);

impl Trigger for Once {
    fn next_fire_time(
        &self,
        previous_fire_time: Option<DateTime<Utc>>,
        _now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        previous_fire_time.map_or(Some(self.0), |_previous| None)
    }
}

#[tokio::test(start_paused = true)]
async fn a_job_whose_trigger_never_fires_again_is_removed() {
    let mut scheduler = scheduler_with(defaults());
    let runs = Arc::new(AtomicUsize::new(0));
    let mut spec = interval_spec("once_job", 10, counting_run(&runs));
    spec.trigger = Box::new(Once(origin()));
    spec.next_run_time = NextRunTime::Explicit(Some(origin()));
    scheduler.add_job(spec).unwrap();
    scheduler.start();
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert!(scheduler.get_job("once_job").is_none());
    scheduler.shutdown();
}
