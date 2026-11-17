#![cfg(test)]

use betterlite_roi::records::ROIReport;

use super::support::{
    drained, inputs, manager, pull, pull_key, pull_param, report_of, settled, until,
};
use crate::roi_cache::store::REPORT_PARAM;

const PULL_FILES: &str = "/repos/o/r/pulls/12/files";

fn numbers_of(report: &ROIReport) -> Vec<i64> {
    report.pulls.iter().map(|record| record.number).collect()
}

#[tokio::test]
async fn results_keep_the_queue_order_when_a_later_pull_finishes_first() {
    let manager = manager(None);
    let inputs = inputs("o/r", &[12, 13, 14]);
    let gate = inputs.routes.gate(PULL_FILES);
    assert!(manager.start(inputs.run()).await);
    until(|| {
        inputs
            .routes
            .requested()
            .iter()
            .any(|path| path == PULL_FILES)
    })
    .await;
    drained().await;
    let later = pull_key(&inputs.settings, "o/r", &pull(13)).await;
    assert!(
        inputs
            .repository
            .written_named(&pull_param(&later))
            .is_some()
    );
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
    gate.notify_one();
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    assert_eq!(numbers_of(&report), vec![12, 13, 14]);
}

#[tokio::test]
async fn results_follow_the_queue_not_the_pull_numbers() {
    let manager = manager(None);
    let inputs = inputs("o/r", &[40, 7, 99]);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    assert_eq!(numbers_of(&report), vec![40, 7, 99]);
    assert_eq!(manager.status().done, 3);
}
