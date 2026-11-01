#![cfg(test)]
use super::fixtures::{capture_progress, claim, never_model, never_read};
use super::orchestrate::run;
use crate::models::execution::{Coverage, Sample};

#[tokio::test]
async fn returns_empty_coverage_findings_and_assessments_without_touching_read_or_model() {
    let (progress, _log) = capture_progress();
    let sample = Sample {
        executions: Vec::new(),
        eligible: 7,
        selected: 0,
        next_offset: None,
        next_cursor: None,
    };
    let result = run(claim(), sample, never_read(), never_model(), progress)
        .await
        .unwrap();
    assert_eq!(
        result.coverage,
        Coverage {
            eligible: 7,
            selected: 0,
            ..Coverage::default()
        }
    );
    assert!(result.findings.is_empty());
    assert!(result.assessments.is_empty());
    assert!(result.error.is_empty());
}

#[tokio::test]
async fn reports_no_progress_at_all() {
    let (progress, log) = capture_progress();
    let sample = Sample {
        executions: Vec::new(),
        eligible: 0,
        selected: 0,
        next_offset: None,
        next_cursor: None,
    };
    run(claim(), sample, never_read(), never_model(), progress)
        .await
        .unwrap();
    assert!(log.lock().unwrap().is_empty());
}
