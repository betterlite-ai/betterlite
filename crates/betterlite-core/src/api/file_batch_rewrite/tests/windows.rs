#![cfg(test)]

use std::collections::BTreeSet;
use std::io::SeekFrom;
use std::iter::once;
use std::sync::atomic::Ordering;

use tokio::sync::Barrier;
use tokio_util::task::TaskTracker;

use super::support::{
    BarrierHook, CountingSource, Probe, ReverseHook, line_of_change, lines_of, ordinal_of, raised,
    timed_scan,
};
use crate::api::file_batch_guardrails::SCAN_WINDOW;
use crate::api::file_batch_rewrite::scan::ScanFile;
use crate::api::file_batch_rewrite::spool::Spool;

// The first window fills to exactly 32 while every hook waits, the hook at line 2 then fails, and the scan stops reading there
#[tokio::test]
async fn window_failure_barrier() {
    let tasks = TaskTracker::new();
    let mut source = CountingSource::new(lines_of(65));
    let failure = raised("r2", 503);
    let hook = BarrierHook {
        probe: Probe::new(source.reads.clone()),
        gate: Barrier::new(SCAN_WINDOW),
        failure: failure.clone(),
        fail_marker: "r2".to_owned(),
    };
    let spool = Spool::default();
    let outcome = timed_scan(&mut source, &hook, &spool, &tasks)
        .await
        .expect("a scan answers");
    match outcome {
        ScanFile::Raised(returned) => assert_eq!(returned, failure),
        other => panic!("the exception at line 2 wins, got {other:?}"),
    }
    let started = hook.probe.started.lock().unwrap();
    assert_eq!(started.len(), 32);
    assert!(started.iter().all(|call| call.reads_before == 32));
    let markers: BTreeSet<String> = started.iter().map(|call| call.marker.clone()).collect();
    let expected_markers: BTreeSet<String> =
        (1_usize..=32).map(|line| format!("r{line}")).collect();
    assert_eq!(markers, expected_markers);
    drop(started);
    assert_eq!(hook.probe.finished.lock().unwrap().len(), 32);
    assert_eq!(source.reads.load(Ordering::SeqCst), 32);
    let seeks = source.seeks.lock().unwrap();
    assert_eq!(seeks.last(), Some(&SeekFrom::Start(0)));
    assert!(spool.is_closed());
}

// Without a failure the file scans in windows of 32, 32 and 1, every hook answers in reverse, and the changes keep the file's order
#[tokio::test]
async fn windows_scan_without_failure() {
    let tasks = TaskTracker::new();
    let mut source = CountingSource::new(lines_of(65));
    let hook = ReverseHook::new(Probe::new(source.reads.clone()), 65);
    let spool = Spool::default();
    let outcome = timed_scan(&mut source, &hook, &spool, &tasks)
        .await
        .expect("a scan answers");
    let ScanFile::Scanned(result) = outcome else {
        panic!("a clean scan reports changes");
    };
    assert_eq!(result.scanned_records, 65);
    let lines: Vec<usize> = result.changes.iter().map(line_of_change).collect();
    assert_eq!(lines, (1_usize..=65).collect::<Vec<usize>>());
    assert_eq!(window_starts(&hook), vec![1_usize, 33, 65]);
    assert_eq!(observed_reads(&hook), expected_reads());
    assert_eq!(hook.probe.max_active.load(Ordering::SeqCst), 32);
    assert_eq!(
        hook.probe.finished.lock().unwrap().clone(),
        reverse_completion()
    );
    assert_eq!(source.reads.load(Ordering::SeqCst), 66);
    let seeks = source.seeks.lock().unwrap();
    assert_eq!(seeks.last(), Some(&SeekFrom::Start(0)));
    assert!(!result.redactions.is_closed());
    assert!(!spool.is_closed());
}

fn window_starts(hook: &ReverseHook) -> Vec<usize> {
    let started = hook.probe.started.lock().unwrap();
    started
        .iter()
        .filter(|call| call.active_before == 0)
        .map(|call| ordinal_of(&call.marker))
        .collect()
}

fn observed_reads(hook: &ReverseHook) -> Vec<usize> {
    hook.probe
        .started
        .lock()
        .unwrap()
        .iter()
        .map(|call| call.reads_before)
        .collect()
}

fn expected_reads() -> Vec<usize> {
    let mut reads = vec![32_usize; 32];
    reads.extend(vec![64_usize; 32]);
    reads.push(66);
    reads
}

fn reverse_completion() -> Vec<String> {
    let first = (1_usize..=32).rev().map(|line| format!("r{line}"));
    let second = (33_usize..=64).rev().map(|line| format!("r{line}"));
    first.chain(second).chain(once("r65".to_owned())).collect()
}
