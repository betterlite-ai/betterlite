#![cfg(test)]

use rstest::rstest;

use super::{cancel_target, list_url, retrieval_url, retrieve_url};
use crate::vertex_batch_lifecycle::error::VertexBatchLifecycleError;

const HOST: &str =
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1";

#[test]
fn default_urls() {
    let retrieve = retrieve_url(None, "proj", "us-central1", "777").unwrap();
    let list = list_url(None, "proj", "us-central1").unwrap();
    let cancel = cancel_target(None, "proj", "us-central1", "777").unwrap();
    assert_eq!(retrieve, format!("{HOST}/batchPredictionJobs/777"));
    assert_eq!(list, format!("{HOST}/batchPredictionJobs"));
    assert_eq!(
        cancel.cancel,
        format!("{HOST}/batchPredictionJobs/777:cancel")
    );
    assert_eq!(cancel.retrieve, retrieve);
}

#[test]
fn the_batch_id_is_one_path_segment() {
    let retrieve = retrieve_url(None, "proj", "us-central1", "a/b:c").unwrap();
    let cancel = cancel_target(None, "proj", "us-central1", "a/b:c").unwrap();
    assert_eq!(retrieve, format!("{HOST}/batchPredictionJobs/a%2Fb%3Ac"));
    assert_eq!(
        cancel.cancel,
        format!("{HOST}/batchPredictionJobs/a%2Fb%3Ac:cancel")
    );
    assert_eq!(cancel.retrieve, retrieve);
}

#[rstest]
#[case::empty("")]
#[case::dot(".")]
#[case::dot_dot("..")]
fn an_empty_or_dot_id_is_refused(#[case] batch_id: &str) {
    let retrieve = retrieve_url(None, "proj", "us-central1", batch_id).unwrap_err();
    let cancel = cancel_target(None, "proj", "us-central1", batch_id).unwrap_err();
    for error in [retrieve, cancel] {
        assert!(matches!(error, VertexBatchLifecycleError::Segment { .. }));
        assert_eq!(error.status_code(), 500);
    }
}

#[rstest]
#[case::host_only(
    "https://proxy.example.com",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/batchPredictionJobs/777",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/batchPredictionJobs",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/batchPredictionJobs/777:cancel",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/batchPredictionJobs/777"
)]
#[case::version_root(
    "https://proxy.example.com/v1",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/batchPredictionJobs/777",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/batchPredictionJobs",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/batchPredictionJobs/777:cancel",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/batchPredictionJobs/777"
)]
#[case::mount_prefix(
    "https://proxy.example.com/gw",
    "https://proxy.example.com/gw://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/batchPredictionJobs/777",
    "https://proxy.example.com/gw://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/batchPredictionJobs",
    "https://proxy.example.com/gw:cancel",
    "https://proxy.example.com/gw"
)]
fn custom_base_urls(
    #[case] api_base: &str,
    #[case] retrieve: &str,
    #[case] list: &str,
    #[case] cancel: &str,
    #[case] after_cancel: &str,
) {
    assert_eq!(
        retrieve_url(Some(api_base), "proj", "us-central1", "777").unwrap(),
        retrieve
    );
    assert_eq!(
        list_url(Some(api_base), "proj", "us-central1").unwrap(),
        list
    );
    let target = cancel_target(Some(api_base), "proj", "us-central1", "777").unwrap();
    assert_eq!(target.cancel, cancel);
    assert_eq!(target.retrieve, after_cancel);
}

#[test]
fn a_terminal_cancel_is_removed_without_touching_slashes() {
    assert_eq!(
        retrieval_url("https://host/jobs/777:cancel"),
        "https://host/jobs/777"
    );
    assert_eq!(
        retrieval_url("https://host/jobs/777/:cancel"),
        "https://host/jobs/777/"
    );
}

#[test]
fn a_cancel_inside_the_url_splits_at_the_last_occurrence_and_trims_slashes() {
    assert_eq!(
        retrieval_url("https://host/jobs/:cancel/extra"),
        "https://host/jobs"
    );
    assert_eq!(
        retrieval_url("https://host/jobs/777"),
        "https://host/jobs/777"
    );
}
