#![cfg(test)]

use std::sync::Arc;

use tokio_util::task::TaskTracker;

use super::support::{
    Answer, CountedBody, FakeFailure, Probe, ScriptedHook, blocked, lines_of, raised, timed_scan,
};
use crate::api::file_batch_guardrails::records::{
    BatchScanFailure, RecordChange, UnparseableRecord,
};
use crate::api::file_batch_rewrite::scan::ScanFile;
use crate::api::file_batch_rewrite::spool::Spool;
use crate::api::file_batch_rewrite::worst::{ScanProblem, worst_problem};

const RAISED_ROWS: [&str; 5] = [
    r#"{"body":7}"#,
    r#"{"body":{"marker":"r2","messages":[]}}"#,
    r#"{"body":{"marker":"r3","messages":[]}}"#,
    r#"{"body":{"marker":"r4","messages":[]}}"#,
    r#"{"body":{"marker":"r5","messages":[]}}"#,
];

const FAILED_ROWS: [&str; 5] = [
    r#"{"body":{"marker":"r1","messages":[]}}"#,
    r#"{"body":7}"#,
    r#"{"body":{"marker":"r3","messages":[]}}"#,
    r#"{"body":{"marker":"r4","messages":[]}}"#,
    r#"{"body":{"x":1}}"#,
];

// A shape failure at line 1 loses to the exception at line 3; two refusals keep the earlier line
#[tokio::test]
async fn worst_problem_priority() {
    match scan_file(
        &RAISED_ROWS,
        vec![
            ("r3", Answer::Raise(raised("r3", 429))),
            ("r5", Answer::Raise(raised("r5", 500))),
        ],
    )
    .await
    {
        ScanFile::Raised(returned) => {
            assert_eq!(returned, raised("r3", 429));
            assert_eq!(returned.status, 429);
        }
        other @ (ScanFile::Scanned(_) | ScanFile::Rejected(_)) => {
            panic!("the exception at line 3 wins, got {other:?}")
        }
    }
    match scan_file(&FAILED_ROWS, Vec::new()).await {
        ScanFile::Rejected(failure) => assert_eq!(
            failure,
            BatchScanFailure::Unparseable(UnparseableRecord { line_number: 2 })
        ),
        other @ (ScanFile::Scanned(_) | ScanFile::Raised(_)) => {
            panic!("the refusal at line 2 wins, got {other:?}")
        }
    }
    // Line 5 raises at once and line 2 raises only after that call returned: the earliest line still wins
    let rows = lines_of(5);
    let row_refs: Vec<&str> = rows.split('\n').collect();
    match scan_file(
        &row_refs,
        vec![
            ("r5", Answer::Raise(raised("r5", 500))),
            ("r2", Answer::RaiseAfter(raised("r2", 503), "r5".to_owned())),
        ],
    )
    .await
    {
        ScanFile::Raised(returned) => assert_eq!(returned, raised("r2", 503)),
        other @ (ScanFile::Scanned(_) | ScanFile::Rejected(_)) => {
            panic!("the exception at line 2 wins, got {other:?}")
        }
    }
}

#[test]
fn worst_problem_prefers_a_later_raised_exception() {
    let problems = vec![
        (7_usize, shape_failure(7)),
        (9, ScanProblem::Raised(raised("r9", 503))),
    ];
    assert_eq!(worst_problem(&problems), Some(1));
    let failures = vec![(5_usize, shape_failure(5)), (2, shape_failure(2))];
    assert_eq!(worst_problem(&failures), Some(1));
    let none: Vec<(usize, ScanProblem<FakeFailure>)> = Vec::new();
    assert_eq!(worst_problem(&none), None);
}

#[tokio::test]
async fn content_blocks_are_changes_not_problems() {
    let tasks = TaskTracker::new();
    let source = CountedBody::new(lines_of(3));
    let hook = ScriptedHook::with(
        vec![
            ("r1", Answer::Block(blocked("g"))),
            ("r2", Answer::Block(blocked("g"))),
            ("r3", Answer::Block(blocked("g"))),
        ],
        Probe::new(Arc::clone(&source.traces.reads)),
    );
    let spool = Spool::default();
    let outcome = timed_scan(&source.body, &hook, &spool, &tasks)
        .await
        .expect("a scan answers");
    let ScanFile::Scanned(result) = outcome else {
        panic!("a block drops a record instead of failing the file");
    };
    assert_eq!(result.changes.len(), 3);
    assert!(
        result
            .changes
            .iter()
            .all(|change| matches!(change, RecordChange::Dropped(_)))
    );
    assert!(
        result
            .changes
            .iter()
            .all(|change| change.guardrail_record().guardrail.as_deref() == Some("g"))
    );
    assert!(!spool.is_closed());
}

fn shape_failure(line: usize) -> ScanProblem<FakeFailure> {
    ScanProblem::Failure(BatchScanFailure::Unparseable(UnparseableRecord {
        line_number: line,
    }))
}

async fn scan_file(rows: &[&str], answers: Vec<(&str, Answer)>) -> ScanFile<Spool, FakeFailure> {
    let tasks = TaskTracker::new();
    let source = CountedBody::new(rows.join("\n"));
    let hook = ScriptedHook::with(answers, Probe::new(Arc::clone(&source.traces.reads)));
    let spool = Spool::default();
    timed_scan(&source.body, &hook, &spool, &tasks)
        .await
        .expect("a scan answers")
}
