use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use tokio::sync::{Semaphore, mpsc as async_mpsc, oneshot};
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
    let workers = Arc::new(Workers {
        slots: Arc::new(Semaphore::new(2)),
        live: Mutex::new(Vec::new()),
    });
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
    workers.shutdown().await.unwrap();
}

// a later submission records its handle without pruning the canceled worker still running, so shutdown joins both
#[tokio::test]
async fn shutdown_joins_a_canceled_worker_after_a_later_submission() {
    let workers = Arc::new(Workers {
        slots: Arc::new(Semaphore::new(2)),
        live: Mutex::new(Vec::new()),
    });
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
    off_runtime(&workers, || {}).await.unwrap();
    let (done, mut done_reading) = oneshot::channel();
    let shutdown_workers = Arc::clone(&workers);
    tasks.spawn(async move {
        shutdown_workers.shutdown().await;
        let _sent = done.send(());
    });
    let early = timeout(Duration::from_millis(200), &mut done_reading).await;
    assert!(early.is_err());
    release.send(()).unwrap();
    timeout(Duration::from_secs(10), &mut done_reading)
        .await
        .unwrap()
        .unwrap();
    assert!(finished.load(Ordering::Relaxed));
}

// shutdown awaits a join thread of its own, so the runtime keeps running other tasks while a canceled worker is still inside its lookup
#[tokio::test]
async fn shutdown_leaves_the_runtime_free_while_the_joins_wait() {
    let workers = Arc::new(Workers {
        slots: Arc::new(Semaphore::new(1)),
        live: Mutex::new(Vec::new()),
    });
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
    let (done, mut done_reading) = oneshot::channel();
    let shutdown_workers = Arc::clone(&workers);
    tasks.spawn(async move {
        shutdown_workers.shutdown().await;
        let _sent = done.send(());
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(done_reading.try_recv().is_err());
    assert!(!finished.load(Ordering::Relaxed));
    release.send(()).unwrap();
    timeout(Duration::from_secs(10), &mut done_reading)
        .await
        .unwrap()
        .unwrap();
    assert!(finished.load(Ordering::Relaxed));
}

// dropping the last owner leaves a canceled worker running: the shutdown path is the one that joins it
#[tokio::test]
async fn workers_drop_does_not_wait_for_a_canceled_worker() {
    let workers = Arc::new(Workers {
        slots: Arc::new(Semaphore::new(1)),
        live: Mutex::new(Vec::new()),
    });
    let (started, mut started_reading) = async_mpsc::channel(1);
    let (release, released_rx) = mpsc::sync_channel::<()>(1);
    let released = Arc::new(Mutex::new(released_rx));
    let (finished, finished_reading) = mpsc::sync_channel::<()>(1);
    let task_workers = Arc::clone(&workers);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        off_runtime(&task_workers, move || {
            let _started = started.try_send(());
            let _released = released
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .recv();
            let _sent = finished.send(());
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
    dropped_reading
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    assert!(matches!(
        finished_reading.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    release.send(()).unwrap();
    dropping.join().unwrap();
    finished_reading
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
}
