#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_roi::records::ROISyncPhase;
use tokio::sync::Barrier;

use super::support::{
    TestClock, barrier_caller, factory, inputs, manager, pull, report_of, settled,
};
use crate::roi_cache::processed::ProcessedPull;
use crate::roi_estimator::Estimator;
use crate::roi_sync::process::{IndexedPull, PullProcessor};
use crate::roi_sync::status::{StatusReporter, idle_status};

const NUMBERS: [i64; 5] = [40, 7, 99, 12, 13];

fn positions(processed: &[ProcessedPull]) -> Vec<usize> {
    processed.iter().map(|pull| pull.position).collect()
}

#[tokio::test]
async fn workers_process_every_third_pull_from_their_offset() {
    let inputs = inputs("o/r", &[40, 7, 99, 12, 13]);
    let queue: Vec<IndexedPull> = NUMBERS
        .iter()
        .enumerate()
        .map(|(position, number)| IndexedPull {
            position,
            repo: "o/r".to_owned(),
            pull: pull(*number),
            key: None,
        })
        .collect();
    let build = factory();
    let github = (build)(&inputs.settings, Some(inputs.routes.clone())).unwrap();
    let estimator = Estimator::new(inputs.settings.clone(), Arc::clone(&inputs.complete), None);
    let clock = TestClock::at("2026-01-10T00:00:00Z");
    let status = StatusReporter {
        stored: Arc::new(Mutex::new(idle_status())),
        clock: clock.handle(),
    };
    let previous = BTreeMap::new();
    let processor = PullProcessor {
        github: &github,
        repository: &inputs.repository,
        estimator: &estimator,
        settings: &inputs.settings,
        estimate_context: "",
        previous: &previous,
        status: &status,
    };
    let first = processor.worker(0, &queue).await.unwrap();
    let second = processor.worker(1, &queue).await.unwrap();
    let third = processor.worker(2, &queue).await.unwrap();
    assert_eq!(positions(&first), vec![0, 3]);
    assert_eq!(positions(&second), vec![1, 4]);
    assert_eq!(positions(&third), vec![2]);
    assert_eq!(status.snapshot().done, 5);
}

#[tokio::test]
async fn workers_run_in_parallel_across_the_queue() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12, 13, 14]);
    inputs.complete = barrier_caller(Arc::new(Barrier::new(3)));
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Complete);
    assert_eq!(status.total, 3);
    assert_eq!(status.estimated, 3);
}

#[tokio::test]
async fn every_pull_is_processed_exactly_once() {
    let manager = manager(None);
    let inputs = inputs("o/r", &[1, 2, 3, 4, 5, 6, 7]);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    let numbers: Vec<i64> = report.pulls.iter().map(|pull| pull.number).collect();
    assert_eq!(numbers, vec![1, 2, 3, 4, 5, 6, 7]);
    let status = manager.status();
    assert_eq!(status.total, 7);
    assert_eq!(status.done, 7);
    assert_eq!(status.estimated, 7);
    assert_eq!(status.reused, 0);
}
