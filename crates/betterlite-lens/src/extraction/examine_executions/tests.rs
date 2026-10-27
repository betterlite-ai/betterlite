#![cfg(test)]
use crate::analysis::{ReadContent, ReportProgress};
use crate::extraction::tests::{model, reader};
use crate::extraction::{
    examine_executions,
    tests::{claim, execution, page},
};
use crate::models::execution::{Coverage, Sample};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::timeout;

#[tokio::test]
async fn concurrent_bounded_progress() {
    let mut input = claim();
    input.job.settings.concurrency = 2;
    let release = Arc::new(Notify::new());
    let started = Arc::new(AtomicUsize::new(0));
    let read = gated_reader(Arc::clone(&started), Arc::clone(&release));
    let coverage = Arc::new(Mutex::new(Vec::new()));
    let progress = reporter(Arc::clone(&coverage), started, release);
    let sample = Sample {
        executions: vec![execution(); 3],
        eligible: 7,
        selected: 9,
        next_offset: None,
        next_cursor: None,
    };
    let results = timeout(
        Duration::from_secs(2),
        examine_executions(input, sample, read, model(vec!["{}"; 3]), progress),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(
        coverage
            .lock()
            .unwrap()
            .iter()
            .map(|item| (item.eligible, item.selected, item.screened))
            .collect::<Vec<_>>(),
        [(7, 3, 0), (7, 3, 1), (7, 3, 2), (7, 3, 3)]
    );
}

#[tokio::test]
async fn progress_empty_sample_reports_once() {
    let sample = Sample {
        executions: vec![],
        eligible: 7,
        selected: 99,
        next_offset: None,
        next_cursor: None,
    };
    let progress: ReportProgress = Arc::new(|stage, coverage: Coverage| {
        assert_eq!(stage, "Reading executions");
        assert_eq!(
            (coverage.eligible, coverage.selected, coverage.screened),
            (7, 0, 0)
        );
        Box::pin(async { Ok(()) })
    });
    let result = examine_executions(claim(), sample, reader(vec![]), model(vec![]), progress)
        .await
        .unwrap();
    assert!(result.is_empty());
}

fn gated_reader(started: Arc<AtomicUsize>, release: Arc<Notify>) -> ReadContent {
    let read: ReadContent = Arc::new(move |_, _, _| {
        let index = started.fetch_add(1, Ordering::SeqCst);
        let wait = Arc::clone(&release);
        Box::pin(async move {
            if index == 1 {
                wait.notified().await;
            }
            Ok(page())
        })
    });
    read
}

fn reporter(
    coverage: Arc<Mutex<Vec<Coverage>>>,
    started: Arc<AtomicUsize>,
    release: Arc<Notify>,
) -> ReportProgress {
    let reported = coverage;
    let progress: ReportProgress = Arc::new(move |stage, current| {
        assert_eq!(stage, "Reading executions");
        if current.screened == 1 {
            assert_eq!(started.load(Ordering::SeqCst), 2);
            release.notify_one();
        }
        reported.lock().unwrap().push(current);
        Box::pin(async { Ok(()) })
    });
    progress
}
