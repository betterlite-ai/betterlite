#![cfg(test)]

use rstest::rstest;

use super::{BatchTarget, BatchTargetInput, vertex_batch_target};
use crate::endpoint_target::error::EndpointTargetError;
use crate::model_urls::error::VertexUrlError;
use crate::vertex_batch_create::error::VertexBatchCreateError;

fn target(api_base: Option<&str>, location: &str) -> Result<BatchTarget, VertexBatchCreateError> {
    vertex_batch_target(&BatchTargetInput {
        api_base,
        project: "proj",
        location,
        model: &format!("projects/proj/locations/{location}/endpoints/222"),
    })
}

#[rstest]
#[case::global(
    None,
    "global",
    "https://aiplatform.googleapis.com/v1/projects/proj/locations/global/batchPredictionJobs",
    "https://aiplatform.googleapis.com/v1/projects/proj/locations/global/endpoints/222"
)]
#[case::regional(
    None,
    "us-central1",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/batchPredictionJobs",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/endpoints/222"
)]
#[case::multi_region(
    None,
    "us",
    "https://aiplatform.us.rep.googleapis.com/v1/projects/proj/locations/us/batchPredictionJobs",
    "https://aiplatform.us.rep.googleapis.com/v1/projects/proj/locations/us/endpoints/222"
)]
#[case::empty_base(
    Some(""),
    "us-central1",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/batchPredictionJobs",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/endpoints/222"
)]
fn collection_global(
    #[case] api_base: Option<&str>,
    #[case] location: &str,
    #[case] collection: &str,
    #[case] endpoint: &str,
) {
    let built = target(api_base, location).unwrap();
    assert_eq!(built.collection, collection);
    assert_eq!(built.endpoint, endpoint);
}

#[rstest]
#[case::v1("https://proxy.example.com/v1", "https://proxy.example.com/v1")]
#[case::v1_slash("https://proxy.example.com/v1/", "https://proxy.example.com/v1")]
#[case::v1beta1(
    "https://proxy.example.com/v1beta1",
    "https://proxy.example.com/v1beta1"
)]
#[case::v1beta1_slash(
    "https://proxy.example.com/v1beta1/",
    "https://proxy.example.com/v1beta1"
)]
#[case::port_and_credentials(
    "http://user:pw@proxy.example.com:8443/v1",
    "http://user:pw@proxy.example.com:8443/v1"
)]
fn version_root_graft(#[case] api_base: &str, #[case] root: &str) {
    let built = target(Some(api_base), "us-central1").unwrap();
    assert_eq!(
        built.collection,
        format!("{root}/projects/proj/locations/us-central1/batchPredictionJobs")
    );
    assert_eq!(
        built.endpoint,
        format!("{root}/projects/proj/locations/us-central1/endpoints/222")
    );
}

#[rstest]
#[case::mount(
    "https://proxy.example.com/gw",
    "https://proxy.example.com/gw://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/batchPredictionJobs",
    "https://proxy.example.com/gw/v1/projects/proj/locations/us-central1/endpoints/222"
)]
#[case::mount_slash(
    "https://proxy.example.com/gw/",
    "https://proxy.example.com/gw/://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/batchPredictionJobs",
    "https://proxy.example.com/gw/v1/projects/proj/locations/us-central1/endpoints/222"
)]
#[case::deeper_than_a_version(
    "https://proxy.example.com/v1/extra",
    "https://proxy.example.com/v1/extra://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/batchPredictionJobs",
    "https://proxy.example.com/v1/extra/v1/projects/proj/locations/us-central1/endpoints/222"
)]
#[case::host_only(
    "https://proxy.example.com",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/batchPredictionJobs",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/endpoints/222"
)]
#[case::host_only_slash(
    "https://proxy.example.com/",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/batchPredictionJobs",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/endpoints/222"
)]
fn mounted_endpoint_get(#[case] api_base: &str, #[case] collection: &str, #[case] endpoint: &str) {
    let built = target(Some(api_base), "us-central1").unwrap();
    assert_eq!(built.collection, collection);
    assert_eq!(built.endpoint, endpoint);
}

#[rstest]
#[case::bad_location(None, "Evil.example")]
#[case::injected_location(None, "x/../y")]
#[case::not_a_url(Some("not a url"), "us-central1")]
#[case::not_http(Some("ftp://proxy.example.com/v1"), "us-central1")]
fn invalid_input_is_refused(#[case] api_base: Option<&str>, #[case] location: &str) {
    let error = target(api_base, location).unwrap_err();
    assert!(matches!(error, VertexBatchCreateError::Target(_)));
    assert_eq!(error.status_code(), 500);
}

#[test]
fn a_bad_location_is_a_vertex_url_error() {
    let error = target(None, "Evil.example").unwrap_err();
    assert!(matches!(
        error,
        VertexBatchCreateError::Target(EndpointTargetError::VertexUrl(
            VertexUrlError::InvalidLocation
        ))
    ));
}
