#![cfg(test)]

use super::fixtures;
use crate::analysis::ReportProgress;
use crate::investigation::coordinator::{Context, investigate_candidates};
use crate::models::execution::Coverage;
use std::sync::{Arc, Mutex};

fn assert_progress(calls: &[(String, Coverage)]) {
    assert_eq!(calls.len(), 2);
    assert!(
        calls
            .iter()
            .all(|(stage, _)| stage == "Checking original evidence")
    );
    assert!(
        calls
            .iter()
            .all(|(_, coverage)| coverage.inconclusive == coverage.investigated)
    );
    let investigated: Vec<i64> = calls
        .iter()
        .map(|(_, coverage)| coverage.investigated)
        .collect();
    assert!(investigated.contains(&1));
    assert!(investigated.contains(&2));
}

#[tokio::test]
async fn investigate_candidates_runs_concurrently_and_reports_progress() {
    let claim = fixtures::claim();
    let mut first = fixtures::candidate(vec![String::from("e1")]);
    first.title = String::from("first candidate");
    let mut second = fixtures::candidate(vec![String::from("e1")]);
    second.title = String::from("second candidate");
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let read = fixtures::never_read();
    let (model, _) = fixtures::scripted(vec![fixtures::inconclusive_decision()]);
    let progress_calls = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&progress_calls);
    let progress: ReportProgress = Arc::new(move |stage, coverage| {
        captured.lock().unwrap().push((stage, coverage));
        Box::pin(async { Ok(()) })
    });

    let context = Context {
        claim,
        candidates: vec![first, second],
        examined,
        read,
        model,
        progress,
        coverage: Coverage::default(),
    };
    let mut investigations = investigate_candidates(context).unwrap();
    let first_result = investigations.next().await.unwrap().unwrap();
    assert!(first_result.finding.is_none());
    assert_eq!(progress_calls.lock().unwrap().len(), 1);
    let second_result = investigations.next().await.unwrap().unwrap();
    assert!(second_result.finding.is_none());
    assert!(investigations.next().await.is_none());
    assert_progress(&progress_calls.lock().unwrap());
}
