use super::super::fixtures::{
    capture_progress, claim, execution, extraction_observation, no_extraction_observation, page,
    part, pipeline_model, reader, submit_decision,
};
use super::super::orchestrate::run;
use crate::analysis::ReadContent;
use crate::models::execution::Sample;

fn two_execution_setup() -> (Sample, ReadContent) {
    let sample = Sample {
        executions: vec![execution("r0"), execution("r1")],
        eligible: 2,
        selected: 2,
        next_offset: None,
        next_cursor: None,
    };
    let read = reader(vec![
        page(part("r0", "s0", "content-a")),
        page(part("r1", "s1", "content-b")),
    ]);
    (sample, read)
}

#[tokio::test]
async fn drives_extraction_clustering_and_investigation_into_a_final_result() {
    let mut input = claim();
    input.job.settings.concurrency = 1;
    let (progress, _log) = capture_progress();
    let (sample, read) = two_execution_setup();
    let extraction = vec![
        extraction_observation("r0", "s0", "expected_behavior", "content-a"),
        no_extraction_observation(),
    ];
    let investigation = vec![submit_decision(
        "r0",
        "s0",
        "expected_behavior",
        "content-a",
    )];
    let model = pipeline_model(extraction, investigation);
    let result = run(input, sample, read, model, progress).await.unwrap();
    assert_eq!(result.findings.len(), 1);
    assert_eq!(result.findings[0].check_id, "expected_behavior");
    assert_eq!(result.findings[0].evidence[0].execution_id, "r0");
    let mut assessed: Vec<_> = result
        .assessments
        .iter()
        .map(|item| item.execution_id.clone())
        .collect();
    assessed.sort();
    assert_eq!(assessed, vec![String::from("r0"), String::from("r1")]);
    assert_eq!(result.coverage.eligible, 2);
    assert_eq!(result.coverage.selected, 2);
    assert_eq!(result.coverage.screened, 2);
    assert_eq!(result.coverage.candidates, 1);
    assert_eq!(result.coverage.investigated, 1);
    assert_eq!(result.coverage.inconclusive, 0);
}
