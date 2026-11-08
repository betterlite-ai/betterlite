#![cfg(test)]

use std::future::pending;
use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::sleep;

use crate::job_scheduler::runner::InFlightRecord;
use crate::job_scheduler::tests::{TokioClock, capture_logs, defaults, origin};
use crate::job_scheduler::triggers::IntervalTrigger;
use crate::job_scheduler::{BoxFuture, JobFn, JobSpec, ProxyScheduler, SchedulerState};
use crate::scheduled_job_stagger::{JobScheduler as _, NextRunTime};

use super::{pause_scheduled_jobs, stop_in_flight_jobs};

fn scheduler() -> ProxyScheduler {
    ProxyScheduler::new(defaults(), Arc::new(TokioClock::new(origin())))
}

fn immediate_spec(id: &str, run: JobFn) -> JobSpec {
    JobSpec {
        id: id.to_owned(),
        trigger: Box::new(IntervalTrigger { seconds: 1 }),
        run,
        replace_existing: false,
        next_run_time: NextRunTime::Explicit(Some(origin())),
        misfire_grace_seconds: None,
        max_instances: None,
    }
}

fn insert_stuck_run(scheduler: &ProxyScheduler, receiver: watch::Receiver<()>) {
    let mut inner = scheduler
        .inner
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    inner.runs.insert(
        7,
        InFlightRecord {
            job_id: "stuck_job".to_owned(),
            finished: receiver,
        },
    );
}

async fn wait_for_in_flight(scheduler: &mut ProxyScheduler, expected: usize) -> usize {
    for _ in 0_u32..200 {
        let running = scheduler.in_flight().len();
        if running >= expected {
            return running;
        }
        sleep(Duration::from_millis(2)).await;
    }
    0
}

#[tokio::test]
async fn pause_scheduled_jobs_only_pauses_a_running_scheduler() {
    let mut scheduler = scheduler();
    pause_scheduled_jobs(&mut scheduler);
    assert_eq!(scheduler.state(), SchedulerState::Stopped);
    scheduler.start();
    pause_scheduled_jobs(&mut scheduler);
    assert_eq!(scheduler.state(), SchedulerState::Paused);
    assert!(scheduler.running());
    scheduler.shutdown();
}

#[tokio::test]
async fn stop_in_flight_jobs_returns_when_the_scheduler_is_stopped() {
    let mut scheduler = scheduler();
    stop_in_flight_jobs(
        &mut scheduler,
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await;
    assert_eq!(scheduler.state(), SchedulerState::Stopped);
}

#[tokio::test]
async fn stop_in_flight_jobs_waits_for_a_job_that_finishes() {
    let mut scheduler = scheduler();
    let finished = Arc::new(AtomicUsize::new(0));
    let shared = Arc::clone(&finished);
    let run: JobFn = Arc::new(move || {
        let slow_counter = Arc::clone(&shared);
        let future: BoxFuture<'static, ()> = Box::pin(async move {
            sleep(Duration::from_millis(50)).await;
            slow_counter.fetch_add(1, Ordering::SeqCst);
        });
        future
    });
    scheduler.add_job(immediate_spec("slow_job", run)).unwrap();
    scheduler.start();
    assert!(wait_for_in_flight(&mut scheduler, 1).await >= 1);
    let (logs, _guard) = capture_logs();
    stop_in_flight_jobs(
        &mut scheduler,
        Duration::from_secs(2),
        Duration::from_secs(1),
    )
    .await;
    assert_eq!(finished.load(Ordering::SeqCst), 1);
    assert!(
        logs.contents()
            .contains("Waiting up to 2s for 1 in-flight scheduled job(s) to finish")
    );
    assert!(!logs.contents().contains("Cancelling"));
}

#[tokio::test]
async fn stop_in_flight_jobs_counts_only_the_jobs_it_cancels() {
    let mut scheduler = scheduler();
    let finished = Arc::new(AtomicUsize::new(0));
    let shared = Arc::clone(&finished);
    let slow: JobFn = Arc::new(move || {
        let slow_counter = Arc::clone(&shared);
        let future: BoxFuture<'static, ()> = Box::pin(async move {
            sleep(Duration::from_millis(50)).await;
            slow_counter.fetch_add(1, Ordering::SeqCst);
        });
        future
    });
    let hanging: JobFn = Arc::new(|| {
        let future: BoxFuture<'static, ()> = Box::pin(pending());
        future
    });
    scheduler.add_job(immediate_spec("slow_job", slow)).unwrap();
    scheduler
        .add_job(immediate_spec("hanging_job", hanging))
        .unwrap();
    scheduler.start();
    assert!(wait_for_in_flight(&mut scheduler, 2).await >= 2);
    let (logs, _guard) = capture_logs();
    stop_in_flight_jobs(
        &mut scheduler,
        Duration::from_secs(2),
        Duration::from_secs(2),
    )
    .await;
    let text = logs.contents();
    assert!(text.contains("Waiting up to 2s for 2 in-flight scheduled job(s) to finish"));
    assert!(text.contains("Cancelling 1 in-flight scheduled job(s) for shutdown"));
    assert_eq!(finished.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stop_in_flight_jobs_waits_while_the_scheduler_is_paused() {
    let mut scheduler = scheduler();
    let finished = Arc::new(AtomicUsize::new(0));
    let shared = Arc::clone(&finished);
    let slow: JobFn = Arc::new(move || {
        let slow_counter = Arc::clone(&shared);
        let future: BoxFuture<'static, ()> = Box::pin(async move {
            sleep(Duration::from_millis(100)).await;
            slow_counter.fetch_add(1, Ordering::SeqCst);
        });
        future
    });
    scheduler
        .add_job(immediate_spec("paused_job", slow))
        .unwrap();
    scheduler.start();
    assert!(wait_for_in_flight(&mut scheduler, 1).await >= 1);
    scheduler.pause();
    assert_eq!(scheduler.state(), SchedulerState::Paused);
    let (logs, _guard) = capture_logs();
    stop_in_flight_jobs(
        &mut scheduler,
        Duration::from_secs(2),
        Duration::from_secs(1),
    )
    .await;
    assert!(
        logs.contents()
            .contains("Waiting up to 2s for 1 in-flight scheduled job(s) to finish")
    );
    assert_eq!(finished.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stop_in_flight_jobs_cancels_a_hanging_job() {
    let mut scheduler = scheduler();
    let run: JobFn = Arc::new(|| {
        let future: BoxFuture<'static, ()> = Box::pin(pending());
        future
    });
    scheduler
        .add_job(immediate_spec("hanging_job", run))
        .unwrap();
    scheduler.start();
    assert!(wait_for_in_flight(&mut scheduler, 1).await >= 1);
    let (logs, _guard) = capture_logs();
    stop_in_flight_jobs(
        &mut scheduler,
        Duration::from_millis(10),
        Duration::from_secs(2),
    )
    .await;
    let text = logs.contents();
    assert!(text.contains("Waiting up to 0.01s for 1 in-flight scheduled job(s) to finish"));
    assert!(text.contains("Cancelling 1 in-flight scheduled job(s) for shutdown"));
    assert!(!text.contains("did not finish within"));
    assert_eq!(scheduler.state(), SchedulerState::Stopped);
}

#[tokio::test]
async fn stop_in_flight_jobs_warns_when_a_cancelled_job_never_finishes() {
    let mut scheduler = scheduler();
    scheduler.start();
    let (sender, receiver) = watch::channel(());
    insert_stuck_run(&scheduler, receiver);
    let (logs, _guard) = capture_logs();
    stop_in_flight_jobs(&mut scheduler, Duration::ZERO, Duration::ZERO).await;
    let text = logs.contents();
    assert!(text.contains("Waiting up to 0s for 1 in-flight scheduled job(s) to finish"));
    assert!(text.contains("Cancelling 1 in-flight scheduled job(s) for shutdown"));
    assert!(text.contains(
        "1 scheduled job(s) did not finish within 0s of cancellation; giving up on them"
    ));
    drop(sender);
}
