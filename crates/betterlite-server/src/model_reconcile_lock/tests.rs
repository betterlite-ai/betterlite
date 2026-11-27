#![cfg(test)]

use std::sync::{Arc, Mutex};

use tokio::task::yield_now;

use super::ModelReconcileLock;

#[tokio::test]
async fn clones_share_one_lock() {
    let lock = ModelReconcileLock::default();
    let other_handle = lock.clone();
    let held = lock.lock().await;
    other_handle.try_lock().unwrap_err();
    drop(held);
    let _reacquired = other_handle.try_lock().unwrap();
}

#[tokio::test]
async fn separate_locks_do_not_block_each_other() {
    let first = ModelReconcileLock::default();
    let second = ModelReconcileLock::default();
    let held = first.lock().await;
    let _second_guard = second.try_lock().unwrap();
    drop(held);
}

#[tokio::test]
async fn waiter_runs_after_the_holder_releases() {
    let lock = ModelReconcileLock::default();
    let record = Arc::new(Mutex::new(Vec::new()));
    let model_lock = lock.clone();
    let search_lock = lock.clone();
    let model_record = Arc::clone(&record);
    let search_record = Arc::clone(&record);

    let model_reconcile = async move {
        let _reconciling = model_lock.lock().await;
        model_record.lock().unwrap().push("model start");
        yield_now().await;
        model_record.lock().unwrap().push("model end");
    };
    let search_refresh = async move {
        let _reconciling = search_lock.lock().await;
        search_record.lock().unwrap().push("search");
    };

    tokio::join!(model_reconcile, search_refresh);

    assert_eq!(
        *record.lock().unwrap(),
        ["model start", "model end", "search"]
    );
    // both holders released the one lock, so the original handle can take it again
    drop(lock.try_lock().unwrap());
}
