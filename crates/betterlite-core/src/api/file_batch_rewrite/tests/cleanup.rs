#![cfg(test)]

use std::io::SeekFrom;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use super::support::{
    Answer, CountedBody, FailingSource, Probe, ScriptedHook, TEST_TIMEOUT, UnrewindableSource,
    lines_of, raised, scan_result, timed_scan,
};
use crate::api::file_batch_guardrails::error::BatchGuardrailError;
use crate::api::file_batch_guardrails::records::{RecordChange, RecordRedacted};
use crate::api::file_batch_rewrite::body::UploadBody;
use crate::api::file_batch_rewrite::error::{RewriteError, ScanFileError};
use crate::api::file_batch_rewrite::rewrite::rewrite_batch_input_file;
use crate::api::file_batch_rewrite::scan::ScanFile;
use crate::api::file_batch_rewrite::spool::{Spool, SpoolError};

// A clean scan still counts every row and closes the spool it filled nothing into; a changed scan hands it back open
#[tokio::test]
async fn scan_and_spool_cleanup() {
    let tasks = TaskTracker::new();
    let source = CountedBody::new(lines_of(3));
    let hook = ScriptedHook::new(Probe::new(Arc::clone(&source.reads)));
    let spool = Spool::default();
    let outcome = timed_scan(&source.body, &hook, &spool, &tasks)
        .await
        .expect("a scan answers");
    let ScanFile::Scanned(result) = outcome else {
        panic!("three clean rows are a scanned file");
    };
    assert_eq!(result.scanned_records, 3);
    assert!(result.changes.is_empty());
    assert!(result.redactions.is_closed());
    assert!(spool.is_closed());
    assert_eq!(
        source.seeks.lock().unwrap().last(),
        Some(&SeekFrom::Start(0))
    );

    let changed_source = CountedBody::new(lines_of(2));
    let changed_hook = ScriptedHook::with(
        vec![("r1", Answer::Rewrite)],
        Probe::new(Arc::clone(&changed_source.reads)),
    );
    let changed_spool = Spool::default();
    let changed = timed_scan(&changed_source.body, &changed_hook, &changed_spool, &tasks)
        .await
        .expect("a scan answers");
    let ScanFile::Scanned(changed_result) = changed else {
        panic!("one rewritten row is a scanned file");
    };
    assert_eq!(changed_result.scanned_records, 2);
    assert_eq!(changed_result.changes.len(), 1);
    assert!(!changed_result.redactions.is_closed());
    assert!(!changed_spool.is_closed());
    let changed_seeks = changed_source.seeks.lock().unwrap();
    assert_eq!(changed_seeks.last(), Some(&SeekFrom::Start(0)));
}

#[tokio::test]
async fn a_parse_failure_closes_the_spool_and_rewinds() {
    let tasks = TaskTracker::new();
    let source = CountedBody::new("not json\n{\"body\":{\"messages\":[]}}".to_owned());
    let hook = ScriptedHook::new(Probe::new(Arc::clone(&source.reads)));
    let spool = Spool::default();
    let outcome = timed_scan(&source.body, &hook, &spool, &tasks).await;
    match outcome {
        Err(ScanFileError::Input(BatchGuardrailError::Record { line_number: 1, .. })) => (),
        other => panic!("the first line is not a record, got {other:?}"),
    }
    assert!(spool.is_closed());
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    let seeks = source.seeks.lock().unwrap();
    assert_eq!(seeks.last(), Some(&SeekFrom::Start(0)));
}

#[tokio::test]
async fn a_hook_failure_closes_the_spool_and_rewinds() {
    let tasks = TaskTracker::new();
    let source = CountedBody::new(lines_of(2));
    let failure = raised("r1", 502);
    let hook = ScriptedHook::with(
        vec![("r1", Answer::Raise(failure.clone()))],
        Probe::new(Arc::clone(&source.reads)),
    );
    let spool = Spool::default();
    let outcome = timed_scan(&source.body, &hook, &spool, &tasks)
        .await
        .expect("a scan answers");
    match outcome {
        ScanFile::Raised(returned) => assert_eq!(returned, failure),
        other @ (ScanFile::Scanned(_) | ScanFile::Rejected(_)) => {
            panic!("the hook's exception aborts the scan, got {other:?}")
        }
    }
    assert!(spool.is_closed());
    let seeks = source.seeks.lock().unwrap();
    assert_eq!(seeks.last(), Some(&SeekFrom::Start(0)));
}

// The spool refuses the rewrite while the scan runs, and the failure still rewinds the source
#[tokio::test]
async fn a_write_failure_rewinds_the_source() {
    let tasks = TaskTracker::new();
    let source = CountedBody::new(lines_of(2));
    let hook = ScriptedHook::with(
        vec![("r1", Answer::Rewrite)],
        Probe::new(Arc::clone(&source.reads)),
    );
    let spool = Spool::default();
    spool.close();
    let outcome = timed_scan(&source.body, &hook, &spool, &tasks).await;
    match outcome {
        Err(ScanFileError::Spool(SpoolError::Closed)) => (),
        other => panic!("a closed spool cannot take a rewrite, got {other:?}"),
    }
    let seeks = source.seeks.lock().unwrap();
    assert_eq!(seeks.last(), Some(&SeekFrom::Start(0)));
}

#[tokio::test]
async fn a_failing_rewind_replaces_the_earlier_result() {
    let tasks = TaskTracker::new();
    let hook = ScriptedHook::new(Probe::new(Arc::new(AtomicUsize::new(0))));
    let spool = Spool::default();
    let unreadable = UploadBody::new(UnrewindableSource::new("not json".to_owned()));
    let outcome = timed_scan(&unreadable, &hook, &spool, &tasks).await;
    match outcome {
        Err(ScanFileError::Rewind { .. }) => (),
        other => panic!("the rewind error replaces the record error, got {other:?}"),
    }
    assert!(spool.is_closed());

    let clean = Spool::default();
    let unreachable = UploadBody::new(UnrewindableSource::new(lines_of(1)));
    let scanned = timed_scan(&unreachable, &hook, &clean, &tasks).await;
    match scanned {
        Err(ScanFileError::Rewind { .. }) => (),
        other => panic!("the rewind error replaces the scan result, got {other:?}"),
    }
}

#[tokio::test]
async fn scan_and_spool_cleanup_cancellation() {
    let tasks = TaskTracker::new();
    let waiting = tasks.clone();
    let spool = Spool::default();
    let observed = spool.clone();
    let reached = Arc::new(Notify::new());
    let cancelled = Arc::new(AtomicUsize::new(0));
    let source = CountedBody::new(lines_of(65));
    let seeks = Arc::clone(&source.seeks);
    let body = source.body.clone();
    let hook = ScriptedHook::cancelling(
        Probe::new(Arc::clone(&source.reads)),
        Arc::clone(&reached),
        Arc::clone(&cancelled),
    );
    let mut scans = JoinSet::new();
    scans.spawn(async move {
        let outcome = timed_scan(&body, &hook, &spool, &tasks).await;
        outcome.map(drop)
    });
    timeout(TEST_TIMEOUT, reached.notified())
        .await
        .expect("the first window starts");
    scans.abort_all();
    scans.join_all().await;
    waiting.close();
    waiting.wait().await;
    assert_eq!(cancelled.load(Ordering::SeqCst), 32);
    assert!(observed.is_closed());
    let recorded = seeks.lock().unwrap();
    assert_eq!(recorded.last(), Some(&SeekFrom::Start(0)));
}

#[tokio::test]
async fn a_rewrite_failure_closes_the_output_and_keeps_the_scan_spool() {
    let tasks = TaskTracker::new();
    let redactions = Spool::default();
    let text = b"{}".to_vec();
    let length = u64::try_from(text.len()).unwrap();
    let offset = redactions.append(&tasks, text).await.unwrap();
    let result = scan_result(
        vec![RecordChange::Redacted(RecordRedacted {
            line_number: 1,
            custom_id: None,
            offset,
            length,
        })],
        1,
        redactions,
    );
    let output = Spool::default();
    let source = FailingSource::default();
    let seeks = Arc::clone(&source.seeks);
    let body = UploadBody::new(source);
    let outcome = rewrite_batch_input_file(&body, &result, &tasks, output.clone()).await;
    match outcome {
        Err(RewriteError::Input(_)) => (),
        other => panic!("the upload cannot be read, got {other:?}"),
    }
    assert!(output.is_closed());
    assert!(!result.redactions.is_closed());
    let recorded = seeks.lock().unwrap();
    assert_eq!(recorded.last(), Some(&SeekFrom::Start(0)));
}
