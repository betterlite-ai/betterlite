#![cfg(test)]
use super::fixtures::{
    capture_progress, claim, execution, model, no_extraction_observation, page, part,
};
use super::orchestrate::run;
use crate::models::execution::Sample;

#[tokio::test]
async fn stops_after_grouping_progress_without_clustering_or_investigating() {
    let (progress, log) = capture_progress();
    let sample = Sample {
        executions: vec![execution("r0")],
        eligible: 1,
        selected: 1,
        next_offset: None,
        next_cursor: None,
    };
    let reader = super::fixtures::reader(vec![page(part("r0", "s0", "nothing notable"))]);
    let model_call = model(vec![no_extraction_observation()]);
    let result = run(claim(), sample, reader, model_call, progress)
        .await
        .unwrap();
    assert!(result.findings.is_empty());
    assert_eq!(result.assessments.len(), 1);
    assert_eq!(
        log.lock().unwrap().last().unwrap().0,
        "Grouping observations"
    );
}
