#![cfg(test)]

use rstest::rstest;

use crate::endpoint_target::graft::graft_default_vertex_path;

#[rstest]
#[case::host_only(
    "https://h.example.com",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent",
    "https://h.example.com/projects/proj/locations/us-central1/publishers/google/models/m:generateContent"
)]
#[case::trailing_slash(
    "https://h.example.com/",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent",
    "https://h.example.com/projects/proj/locations/us-central1/publishers/google/models/m:generateContent"
)]
#[case::version_root(
    "https://h.example.com/v1",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:streamGenerateContent?alt=sse",
    "https://h.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:streamGenerateContent"
)]
#[case::version_root_v1beta1(
    "https://h.example.com/v1beta1",
    "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent",
    "https://h.example.com/v1beta1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent"
)]
#[case::query_preserved(
    "https://h.example.com/v1?x=1",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent",
    "https://h.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent?x=1"
)]
#[case::base_path(
    "https://h.example.com/base",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent",
    "https://h.example.com/base/projects/proj/locations/us-central1/publishers/google/models/m:generateContent"
)]
fn graft_default_vertex_path_cases(
    #[case] api_base: &str,
    #[case] default_url: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        graft_default_vertex_path(api_base, default_url).unwrap(),
        expected
    );
}
