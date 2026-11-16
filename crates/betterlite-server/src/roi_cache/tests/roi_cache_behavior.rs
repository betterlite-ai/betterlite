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
    let settings = settings();
    let batch = read_repositories(
        &github,
        &settings.repos,
        day("2026-01-01"),
        day("2026-01-31"),
    )
    .await
    .unwrap();
    assert_eq!(batch.stage, stage);
    assert_eq!(batch.warnings.len(), warnings);

    let repository = MemoryRepository::holding("roi_calculator_report", &report());
    let previous = previous_report(&repository).await.unwrap().unwrap();
    let mut records = Vec::new();
    for (position, (repo, pull)) in batch.queue.iter().enumerate() {
        let key = cache_key(&settings, "ctx", repo, pull).unwrap();
        let built = report_record(
            &evidence(),
            estimate(ROIEstimateStatus::Estimated),
            key.clone(),
        );
        cache_estimated_pull(&repository, key.as_deref(), &built, None)
            .await
            .unwrap();
        records.push(processed(position, built.estimate.status, false));
    }
    assert_eq!(processed_records(records).unwrap().len(), batch.queue.len());
    assert_eq!(repository.written().len(), batch.queue.len());
    let (name, _) = repository.written().remove(0);
    let key = cache_key(&settings, "ctx", "a/b", &batch.queue[0].1)
        .unwrap()
        .unwrap();
    assert_eq!(name, format!("roi_calculator_pull_{key}"));
    let reused = cached_record(&previous.pulls[0]);
    assert_eq!(reused.estimate.cached, Some(true));
}
