#![cfg(test)]

use betterlite_roi::records::ROIEstimateStatus;
use rstest::rstest;

use crate::roi_cache::key::cache_key;
use crate::roi_cache::processed::processed_records;
use crate::roi_cache::records::{cached_record, report_record};
use crate::roi_cache::repositories::read_repositories;
use crate::roi_cache::store::{cache_estimated_pull, previous_report};

use super::support::{
    MemoryRepository, Routes, day, estimate, evidence, github, processed, pulls_body, report,
    settings,
};

#[rstest]
#[case::every_repository_readable(true, "Analysis complete", 0)]
#[case::one_repository_unreadable(false, "Analysis complete with unavailable repositories", 1)]
#[tokio::test]
async fn roi_cache_behavior(
    #[case] second_readable: bool,
    #[case] stage: &str,
    #[case] warnings: usize,
) {
    let mut routes = Routes::default().with("/repos/a/b/pulls", 200, &pulls_body(&[1]));
    if second_readable {
        routes = routes.with("/repos/c/d/pulls", 200, &pulls_body(&[2]));
    }
    let github = github(routes);
    let selected = settings();
    let batch = read_repositories(
        &github,
        &selected.repos,
        day("2026-01-01"),
        day("2026-01-31"),
    )
    .await
    .unwrap();
    assert_eq!(batch.stage, stage);
    assert_eq!(batch.warnings.len(), warnings);

    let repository = MemoryRepository::holding("roi_calculator_report", &report());
    let previous = previous_report(&repository).await.unwrap().unwrap();
    let (first_repo, first_pull) = batch.queue.first().unwrap();
    let key = cache_key(&selected, "ctx", first_repo, first_pull).unwrap();
    let built = report_record(
        &evidence(),
        estimate(ROIEstimateStatus::Estimated),
        key.clone(),
    );
    cache_estimated_pull(&repository, key.as_deref(), &built, None)
        .await
        .unwrap();
    let written = repository.written();
    let expected_name = format!("roi_calculator_pull_{}", key.unwrap());
    assert_eq!(
        written
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<&str>>(),
        vec![expected_name.as_str()]
    );

    let records: Vec<_> = batch
        .queue
        .iter()
        .enumerate()
        .map(|(position, _)| processed(position, ROIEstimateStatus::Estimated, false))
        .collect();
    assert_eq!(processed_records(records).unwrap().len(), batch.queue.len());
    let reused = cached_record(previous.pulls.first().unwrap());
    assert_eq!(reused.estimate.cached, Some(true));
}
