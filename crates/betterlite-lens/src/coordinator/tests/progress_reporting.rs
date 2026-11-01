#![cfg(test)]
use super::fixtures::{
    capture_progress, claim, execution, extraction_observation, no_extraction_observation, page,
    part, pipeline_model, reader, submit_decision,
};
use super::orchestrate::run;
use crate::models::execution::{Coverage, Sample};

async fn recorded_stages() -> Vec<(String, Coverage)> {
    let mut input = claim();
    input.job.settings.concurrency = 1;
    let (progress, log) = capture_progress();
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
    run(input, sample, read, model, progress).await.unwrap();
    log.lock().unwrap().clone()
}

fn assert_reading_stage(recorded: &[(String, Coverage)]) {
    let reading: Vec<_> = recorded
        .iter()
        .filter(|(stage, _)| stage == "Reading executions")
        .collect();
    assert_eq!(reading.len(), 3);
    assert_eq!(
        reading
            .iter()
            .map(|(_, coverage)| coverage.screened)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(
        reading
            .iter()
            .all(|(_, coverage)| coverage.eligible == 2 && coverage.selected == 2)
    );
}

fn assert_grouping_stage(recorded: &[(String, Coverage)]) {
    let grouping: Vec<_> = recorded
        .iter()
        .filter(|(stage, _)| stage == "Grouping observations")
        .collect();
    assert_eq!(grouping.len(), 2);
    assert_eq!(grouping[0].1.screened, 2);
    assert_eq!(grouping[0].1.partial, 0);
    assert_eq!(grouping[0].1.unassessable, 0);
    assert_eq!(grouping[1].1.grouping_batches, 1);
    assert_eq!(grouping[1].1.grouped_batches, 0);
}

fn assert_checking_stage(recorded: &[(String, Coverage)]) {
    let checking: Vec<_> = recorded
        .iter()
        .filter(|(stage, _)| stage == "Checking original evidence")
        .collect();
    assert_eq!(checking.len(), 1);
    assert_eq!(checking[0].1.candidates, 1);
    assert_eq!(checking[0].1.investigated, 1);
    assert_eq!(checking[0].1.inconclusive, 0);
}

#[tokio::test]
async fn reports_every_stage_with_the_matching_coverage_fields() {
    let recorded = recorded_stages().await;
    assert_reading_stage(&recorded);
    assert_grouping_stage(&recorded);
    assert_checking_stage(&recorded);
}
