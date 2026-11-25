use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use tokio::sync::mpsc as async_mpsc;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::db_config_overlay::environ::{Workers, off_runtime};
use crate::db_config_overlay::error::OverlayError;

// one submitted lookup that records its start and waits for the test's release token
fn spawn_lookup(
    tasks: &mut JoinSet<Result<(), OverlayError>>,
    workers: &Arc<Workers>,
    started: &async_mpsc::Sender<()>,
    released: &Arc<Mutex<mpsc::Receiver<()>>>,
    count: &Arc<AtomicUsize>,
) {
    let task_workers = Arc::clone(workers);
    let task_started = started.clone();
    let task_released = Arc::clone(released);
    let task_count = Arc::clone(count);
    tasks.spawn(async move {
        off_runtime(&task_workers, move || {
            task_count.fetch_add(1, Ordering::Relaxed);
            let _started = task_started.try_send(());
            let _released = task_released
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .recv();
        })
        .await
    });
}

// two permits, four submissions: the last two wait for a worker to finish
#[tokio::test]
async fn workers_bound_the_live_threads() {
    let workers = Arc::new(Workers::new(2));
    let (started, mut started_reading) = async_mpsc::channel(8);
    let (release, released_rx) = mpsc::sync_channel::<()>(4);
    let released = Arc::new(Mutex::new(released_rx));
    let count = Arc::new(AtomicUsize::new(0));
    let mut tasks = JoinSet::new();
    spawn_lookup(&mut tasks, &workers, &started, &released, &count);
    spawn_lookup(&mut tasks, &workers, &started, &released, &count);
    spawn_lookup(&mut tasks, &workers, &started, &released, &count);
    spawn_lookup(&mut tasks, &workers, &started, &released, &count);
    started_reading.recv().await.unwrap();
    started_reading.recv().await.unwrap();
    let early = timeout(Duration::from_millis(200), started_reading.recv()).await;
    assert_eq!(early.unwrap_or(None), None);
    release.send(()).unwrap();
    release.send(()).unwrap();
    release.send(()).unwrap();
    release.send(()).unwrap();
    let joined = timeout(Duration::from_secs(10), tasks.join_all())
        .await
        .unwrap();
    assert_eq!(joined.len(), 4);
    assert_eq!(count.load(Ordering::Relaxed), 4);
}

// the future is gone, but its worker still runs, and the owner's drop waits for it
#[tokio::test]
async fn workers_join_a_canceled_worker_when_the_owner_drops() {
    let workers = Arc::new(Workers::new(1));
    let (started, mut started_reading) = async_mpsc::channel(1);
    let (release, released_rx) = mpsc::sync_channel::<()>(1);
    let released = Arc::new(Mutex::new(released_rx));
    let finished = Arc::new(AtomicBool::new(false));
    let task_workers = Arc::clone(&workers);
    let task_finished = Arc::clone(&finished);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        off_runtime(&task_workers, move || {
            let _started = started.try_send(());
            let _released = released
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .recv();
            task_finished.store(true, Ordering::Relaxed);
        })
        .await
    });
    started_reading.recv().await.unwrap();
    tasks.abort_all();
    let canceled = tasks.join_next().await.unwrap().unwrap_err();
    assert!(canceled.is_cancelled());
    let (dropped, dropped_reading) = mpsc::sync_channel::<()>(1);
    let dropping = thread::Builder::new()
        .name("b8379-drop-workers".to_owned())
        .spawn(move || {
            drop(workers);
            let _sent = dropped.send(());
        })
        .unwrap();
    let early = dropped_reading.recv_timeout(Duration::from_millis(200));
    assert!(matches!(early, Err(RecvTimeoutError::Timeout)));
    release.send(()).unwrap();
    dropped_reading
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    dropping.join().unwrap();
    assert!(finished.load(Ordering::Relaxed));
}
