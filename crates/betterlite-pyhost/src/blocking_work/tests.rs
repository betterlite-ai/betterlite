#![cfg(test)]

use std::io;
use std::iter::repeat_with;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;
use tokio::task::{JoinSet, yield_now};
use tokio::time::advance;
use tracing::subscriber::{DefaultGuard, set_default};

use super::{Stats, WorkError, WorkTracker};
use crate::host::{HostConfig, PythonHost};

const LONG_DEADLINE: Duration = Duration::from_secs(3600);
const WAIT_LIMIT: Duration = Duration::from_secs(10);

fn start_host() -> PythonHost {
    PythonHost::start(&HostConfig::default()).unwrap()
}

async fn wait_until_started(started: &AtomicBool) {
    let clock = Instant::now();
    while !started.load(Ordering::SeqCst) {
        assert!(clock.elapsed() < WAIT_LIMIT, "the Python job never started");
        yield_now().await;
    }
}

async fn wait_for_started_count(started: &AtomicUsize, expected: usize) {
    let clock = Instant::now();
    while started.load(Ordering::SeqCst) < expected {
        assert!(
            clock.elapsed() < WAIT_LIMIT,
            "the pool did not start {expected} jobs"
        );
        yield_now().await;
    }
}

async fn wait_for_idle(tracker: &WorkTracker) {
    let clock = Instant::now();
    while tracker.stats().running != 0 {
        assert!(clock.elapsed() < WAIT_LIMIT, "the pool never became idle");
        yield_now().await;
    }
}

async fn yield_times(count: usize) {
    for _ in 0..count {
        yield_now().await;
    }
}

fn release_channels(count: usize) -> (Vec<oneshot::Sender<()>>, Vec<oneshot::Receiver<()>>) {
    repeat_with(oneshot::channel::<()>).take(count).unzip()
}

fn spawn_blocked_jobs(
    tasks: &mut JoinSet<Result<(), WorkError>>,
    tracker: &Arc<WorkTracker>,
    host: &PythonHost,
    started: &Arc<AtomicUsize>,
    releases: Vec<oneshot::Receiver<()>>,
) {
    for release in releases {
        let job_tracker = Arc::clone(tracker);
        let job_host = host.clone();
        let job_started = Arc::clone(started);
        tasks.spawn(async move {
            job_tracker
                .run(&job_host, LONG_DEADLINE, move |_py| {
                    job_started.fetch_add(1, Ordering::SeqCst);
                    release.blocking_recv().unwrap_or_default();
                })
                .await
        });
    }
}

fn spawn_one_blocked_job<T>(
    tracker: &Arc<WorkTracker>,
    host: &PythonHost,
    deadline: Duration,
    value: T,
) -> (
    JoinSet<Result<T, WorkError>>,
    oneshot::Sender<()>,
    Arc<AtomicBool>,
)
where
    T: Send + 'static,
{
    let started = Arc::new(AtomicBool::new(false));
    let job_started = Arc::clone(&started);
    let (release_sender, release_receiver) = oneshot::channel();
    let job_tracker = Arc::clone(tracker);
    let job_host = host.clone();
    let mut runs = JoinSet::new();
    runs.spawn(async move {
        job_tracker
            .run(&job_host, deadline, move |_py| {
                job_started.store(true, Ordering::SeqCst);
                release_receiver.blocking_recv().unwrap_or_default();
                value
            })
            .await
    });
    (runs, release_sender, started)
}

async fn join_one<T>(tasks: &mut JoinSet<Result<T, WorkError>>) -> Result<T, WorkError>
where
    T: 'static,
{
    let clock = Instant::now();
    loop {
        if let Some(result) = tasks.try_join_next() {
            return result.unwrap();
        }
        assert!(
            clock.elapsed() < WAIT_LIMIT,
            "the worker task did not finish"
        );
        yield_now().await;
    }
}

async fn join_all<T>(tasks: &mut JoinSet<Result<T, WorkError>>, expected: usize)
where
    T: 'static,
{
    let clock = Instant::now();
    let mut joined = 0_usize;
    while joined < expected {
        if let Some(result) = tasks.try_join_next() {
            result.unwrap().unwrap();
            joined = joined.saturating_add(1);
            continue;
        }
        assert!(
            clock.elapsed() < WAIT_LIMIT,
            "the pool did not finish its jobs"
        );
        yield_now().await;
    }
}

async fn join_quiet(tasks: &mut JoinSet<()>) {
    let clock = Instant::now();
    loop {
        if let Some(result) = tasks.try_join_next() {
            result.unwrap();
            return;
        }
        assert!(
            clock.elapsed() < WAIT_LIMIT,
            "the shutdown task did not finish"
        );
        yield_now().await;
    }
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

#[tokio::test]
async fn run_returns_the_job_result() {
    let host = start_host();
    let tracker = WorkTracker::new(2);
    let result = tracker
        .run(&host, Duration::from_secs(5), |py| py.version_info().major)
        .await
        .unwrap();
    assert_eq!(result, 3);
    assert_eq!(
        tracker.stats(),
        Stats {
            running: 0,
            abandoned: 0,
            completed: 1
        }
    );
}

#[tokio::test]
async fn a_panicking_job_gives_panicked() {
    let host = start_host();
    let tracker = WorkTracker::new(2);
    let outcome = tracker
        .run(&host, Duration::from_secs(5), |_py| -> u8 {
            panic!("the hook panicked")
        })
        .await;
    match outcome {
        Err(WorkError::Panicked(message)) => {
            assert!(message.contains("the hook panicked"), "got: {message}");
        }
        Ok(value) => panic!("a panicking job must not return, got {value}"),
        Err(other) => panic!("a panicking job must give Panicked, got {other:?}"),
    }
    assert_eq!(
        tracker.stats(),
        Stats {
            running: 0,
            abandoned: 0,
            completed: 1
        }
    );
}

#[tokio::test(start_paused = true)]
async fn run_times_out_at_the_deadline_and_counts_the_job_abandoned() {
    let host = start_host();
    let tracker = Arc::new(WorkTracker::new(2));
    let (mut runs, release_sender, started) =
        spawn_one_blocked_job(&tracker, &host, Duration::from_secs(5), 7_u8);
    wait_until_started(&started).await;
    advance(Duration::from_secs(5)).await;
    match join_one(&mut runs).await {
        Err(WorkError::TimedOut { after }) => assert_eq!(after, Duration::from_secs(5)),
        Ok(value) => panic!("the job must not finish before its deadline, got {value}"),
        Err(other) => panic!("the job must time out, got {other:?}"),
    }
    assert_eq!(
        tracker.stats(),
        Stats {
            running: 1,
            abandoned: 1,
            completed: 0
        }
    );
    release_sender.send(()).unwrap();
    wait_for_idle(&tracker).await;
    assert_eq!(
        tracker.stats(),
        Stats {
            running: 0,
            abandoned: 0,
            completed: 1
        }
    );
}

#[tokio::test(start_paused = true)]
async fn the_pool_never_runs_more_than_max_threads_jobs() {
    let host = start_host();
    let tracker = Arc::new(WorkTracker::new(2));
    let started = Arc::new(AtomicUsize::new(0));
    let (releases, receivers) = release_channels(3);
    let mut tasks = JoinSet::new();
    spawn_blocked_jobs(&mut tasks, &tracker, &host, &started, receivers);
    wait_for_started_count(&started, 2).await;
    yield_times(100).await;
    assert_eq!(started.load(Ordering::SeqCst), 2);
    assert_eq!(tracker.stats().running, 2);
    drop(releases);
    join_all(&mut tasks, 3).await;
    assert_eq!(
        tracker.stats(),
        Stats {
            running: 0,
            abandoned: 0,
            completed: 3
        }
    );
}

#[tokio::test(start_paused = true)]
async fn the_default_pool_runs_sixteen_jobs_at_once() {
    let host = start_host();
    let tracker = Arc::new(WorkTracker::default());
    let started = Arc::new(AtomicUsize::new(0));
    let (releases, receivers) = release_channels(17);
    let mut tasks = JoinSet::new();
    spawn_blocked_jobs(&mut tasks, &tracker, &host, &started, receivers);
    wait_for_started_count(&started, 16).await;
    yield_times(100).await;
    assert_eq!(started.load(Ordering::SeqCst), 16);
    assert_eq!(tracker.stats().running, 16);
    drop(releases);
    join_all(&mut tasks, 17).await;
    assert_eq!(
        tracker.stats(),
        Stats {
            running: 0,
            abandoned: 0,
            completed: 17
        }
    );
}

#[tokio::test(start_paused = true)]
async fn shutdown_waits_for_running_jobs() {
    let host = start_host();
    let tracker = Arc::new(WorkTracker::new(2));
    let (mut runs, release_sender, started) =
        spawn_one_blocked_job(&tracker, &host, LONG_DEADLINE, ());
    wait_until_started(&started).await;
    let shutdown_tracker = Arc::clone(&tracker);
    let mut shutdowns = JoinSet::new();
    shutdowns.spawn(async move { shutdown_tracker.shutdown().await });
    yield_times(100).await;
    assert!(
        shutdowns.try_join_next().is_none(),
        "shutdown returned while a Python job was still running"
    );
    let refused = tracker.run(&host, Duration::from_secs(5), |_py| 0_u8).await;
    assert!(matches!(refused, Err(WorkError::ShuttingDown)));
    release_sender.send(()).unwrap();
    join_quiet(&mut shutdowns).await;
    join_one(&mut runs).await.unwrap();
    assert_eq!(
        tracker.stats(),
        Stats {
            running: 0,
            abandoned: 0,
            completed: 1
        }
    );
}

#[tokio::test(start_paused = true)]
async fn shutdown_warns_after_the_grace_with_jobs_still_running() {
    let host = start_host();
    let tracker = Arc::new(WorkTracker::new(2));
    let (mut runs, release_sender, started) =
        spawn_one_blocked_job(&tracker, &host, LONG_DEADLINE, ());
    wait_until_started(&started).await;
    let (captured, _guard) = capture();
    tracker.shutdown().await;
    let text = captured.text();
    assert!(text.contains("shutdown grace elapsed"), "logs were: {text}");
    assert!(text.contains("jobs=1"), "logs were: {text}");
    assert_eq!(tracker.stats().running, 1);
    release_sender.send(()).unwrap();
    wait_for_idle(&tracker).await;
    join_one(&mut runs).await.unwrap();
    assert_eq!(
        tracker.stats(),
        Stats {
            running: 0,
            abandoned: 0,
            completed: 1
        }
    );
}
