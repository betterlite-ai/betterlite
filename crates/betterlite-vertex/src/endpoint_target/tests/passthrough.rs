#![cfg(test)]

use rstest::rstest;

use crate::endpoint_target::error::EndpointTargetError;
use crate::endpoint_target::passthrough::{
    api_version_for_route, construct_target_url, get_vertex_location_from_url,
    get_vertex_model_id_from_url, get_vertex_project_id_from_url, replace_route_project_location,
    with_api_version,
};
use crate::model_urls::ApiVersion;

#[rstest]
#[case::complete_url(
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj-1/locations/us-central1/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse",
    Some("proj-1"),
    Some("us-central1"),
    Some("gemini-2.5-flash")
)]
#[case::cached_content(
    "https://h.example.com/v1beta1/projects/p/locations/l/cachedContents/abc",
    Some("p"),
    Some("l"),
    None
)]
#[case::model_with_slash(
    "https://h.example.com/x/models/a/b:generateContent",
    None,
    None,
    Some("a/b")
)]
#[case::no_markers("https://h.example.com/nothing", None, None, None)]
#[case::empty_segments(
    "https://h.example.com/projects//locations//models/:x",
    None,
    None,
    None
)]
#[case::first_match(
    "https://h.example.com/projects/p/locations/l/x/projects/q/locations/r",
    Some("p"),
    Some("l"),
    None
)]
#[case::nested_models(
    "https://h.example.com/v1/projects/p/locations/l/publishers/google/models/a/models/b:predict",
    Some("p"),
    Some("l"),
    Some("a/models/b")
)]
fn passthrough_extraction_cases(
    #[case] url: &str,
    #[case] project: Option<&str>,
    #[case] location: Option<&str>,
    #[case] model: Option<&str>,
) {
    assert_eq!(get_vertex_project_id_from_url(url), project);
    assert_eq!(get_vertex_location_from_url(url), location);
    assert_eq!(get_vertex_model_id_from_url(url), model);
}

#[rstest]
#[case::single("/projects/a/locations/b/x", "/projects/p/locations/l/x")]
#[case::multiple(
    "/projects/a/locations/b/x/projects/c/locations/d/y",
    "/projects/p/locations/l/x/projects/p/locations/l/y"
)]
#[case::no_trailing_slash("/projects/a/locations/b", "/projects/a/locations/b")]
#[case::empty_location("/projects/a/locations//x", "/projects/a/locations//x")]
#[case::empty_project("/projects//locations/l/x", "/projects//locations/l/x")]
#[case::project_with_slash("/projects/a/b/locations/l/x", "/projects/a/b/locations/l/x")]
#[case::overlapping_marker(
    "/projects/projects/x/locations/y/z",
    "/projects/projects/p/locations/l/z"
)]
#[case::not_a_route("/x/projects/a/locations/b/", "/x/projects/p/locations/l/")]
fn replace_route_project_location_cases(#[case] route: &str, #[case] expected: &str) {
    assert_eq!(
        replace_route_project_location(route, "p", "l").unwrap(),
        expected
    );
}

#[rstest]
#[case::plain("/publishers/google/models/m:generateContent", ApiVersion::V1)]
#[case::cached_content("/projects/p/locations/l/cachedContent", ApiVersion::V1Beta1)]
#[case::cached_contents("/projects/p/locations/l/cachedContents", ApiVersion::V1Beta1)]
#[case::case_sensitive("/projects/p/locations/l/CachedContent", ApiVersion::V1)]
fn api_version_for_route_cases(#[case] route: &str, #[case] expected: ApiVersion) {
    assert_eq!(api_version_for_route(route), expected);
}

#[rstest]
#[case::project_route("/projects/a/x", "/v1/projects/a/x")]
#[case::cached_project_route("/projects/a/cachedContent", "/v1beta1/projects/a/cachedContent")]
#[case::other_route("/publishers/a", "/publishers/a")]
fn with_api_version_cases(#[case] route: &str, #[case] expected: &str) {
    assert_eq!(with_api_version(route), expected);
}

#[rstest]
#[case::replace_both(
    "https://us-central1-aiplatform.googleapis.com",
    "/projects/old/locations/oldloc/publishers/google/models/gemini-2.5-flash:generateContent",
    Some("newproj"),
    Some("newloc"),
    "https://us-central1-aiplatform.googleapis.com/v1/projects/newproj/locations/newloc/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::missing_project_keeps_route(
    "https://us-central1-aiplatform.googleapis.com",
    "/projects/old/locations/oldloc/publishers/google/models/gemini-2.5-flash:generateContent",
    None,
    Some("newloc"),
    "https://us-central1-aiplatform.googleapis.com/v1/projects/old/locations/oldloc/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::empty_location_keeps_route(
    "https://us-central1-aiplatform.googleapis.com",
    "/projects/old/locations/oldloc/publishers/google/models/gemini-2.5-flash:generateContent",
    Some("newproj"),
    Some(""),
    "https://us-central1-aiplatform.googleapis.com/v1/projects/old/locations/oldloc/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::locations_route_without_projects_prefix(
    "https://h.example.com",
    "/some/locations/x",
    Some("p"),
    Some("l"),
    "https://h.example.com/some/locations/x"
)]
#[case::explicit_v1(
    "https://h.example.com",
    "/v1/publishers/google/models/m:generateContent",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1/projects/p/locations/l/publishers/google/models/m:generateContent"
)]
#[case::explicit_v1beta1(
    "https://h.example.com",
    "/v1beta1/publishers/google/models/m:generateContent",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1beta1/projects/p/locations/l/publishers/google/models/m:generateContent"
)]
#[case::default_version(
    "https://h.example.com",
    "/publishers/google/models/m:generateContent",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1/projects/p/locations/l/publishers/google/models/m:generateContent"
)]
#[case::cached_content_default_version(
    "https://h.example.com",
    "/cachedContents",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1beta1/projects/p/locations/l/cachedContents"
)]
#[case::cached_content_route_with_locations(
    "https://h.example.com",
    "/projects/old/locations/old/cachedContents",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1beta1/projects/p/locations/l/cachedContents"
)]
#[case::explicit_version_wins_over_cache(
    "https://h.example.com",
    "/v1/projects/old/locations/old/cachedContents",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1/projects/p/locations/l/cachedContents"
)]
#[case::route_without_leading_slash(
    "https://h.example.com",
    "publishers/google/models/m:generateContent",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1/projects/p/locations/lpublishers/google/models/m:generateContent"
)]
#[case::base_with_query(
    "https://proxy.example.com/base?x=1",
    "/publishers/google/models/m:generateContent",
    Some("p"),
    Some("l"),
    "https://proxy.example.com/v1/projects/p/locations/l/publishers/google/models/m:generateContent?x=1"
)]
#[case::base_with_trailing_slash(
    "https://h.example.com/",
    "/publishers/google/models/m:generateContent",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1/projects/p/locations/l/publishers/google/models/m:generateContent"
)]
#[case::base_with_fragment(
    "https://h.example.com/base#frag",
    "/publishers/google/models/m:generateContent",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1/projects/p/locations/l/publishers/google/models/m:generateContent#frag"
)]
#[case::overlapping_marker(
    "https://h.example.com",
    "/projects/projects/x/locations/y/z",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1/projects/projects/p/locations/l/z"
)]
fn construct_target_url_cases(
    #[case] base_url: &str,
    #[case] route: &str,
    #[case] project: Option<&str>,
    #[case] location: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(
        construct_target_url(base_url, route, project, location).unwrap(),
        expected
    );
}

#[test]
fn construct_target_url_requires_project_and_location_without_a_locations_route() {
    let base = "https://h.example.com";
    let route = "/publishers/google/models/m:generateContent";
    assert!(matches!(
        construct_target_url(base, route, None, Some("l")).unwrap_err(),
        EndpointTargetError::Project
    ));
    assert!(matches!(
        construct_target_url(base, route, Some("p"), None).unwrap_err(),
        EndpointTargetError::Location
    ));
    assert!(matches!(
        construct_target_url(base, route, Some(""), Some("l")).unwrap_err(),
        EndpointTargetError::Project
    ));
}

#[test]
fn construct_target_url_rejects_a_bad_base_or_route() {
    assert!(matches!(
        construct_target_url("not a url", "/x", Some("p"), Some("l")).unwrap_err(),
        EndpointTargetError::BaseUrl { .. }
    ));
    assert!(matches!(
        construct_target_url("ftp://h.example.com", "/x", Some("p"), Some("l")).unwrap_err(),
        EndpointTargetError::BaseScheme
    ));
    assert!(matches!(
        construct_target_url("https://h.example.com", "/x?y=1", Some("p"), Some("l")).unwrap_err(),
        EndpointTargetError::Route
    ));
    assert!(matches!(
        construct_target_url("https://h.example.com", "/x#f", Some("p"), Some("l")).unwrap_err(),
        EndpointTargetError::Route
    ));
    assert!(matches!(
        construct_target_url("https://h.example.com", "/x\ty", Some("p"), Some("l")).unwrap_err(),
        EndpointTargetError::Route
    ));
}

#[test]
fn construct_target_url_normalizes_dot_segments() {
    assert_eq!(
        construct_target_url(
            "https://h.example.com/base",
            "/x/../y",
            Some("p"),
            Some("l")
        )
        .unwrap(),
        "https://h.example.com/v1/projects/p/locations/l/y"
    );
    assert_eq!(
        construct_target_url("https://h.example.com/base", "/../x", Some("p"), Some("l")).unwrap(),
        "https://h.example.com/v1/projects/p/locations/x"
    );
}

#[test]
fn construct_target_url_keeps_the_destination_authority() {
    let url = construct_target_url(
        "https://h.example.com",
        "//evil.example.com/x",
        Some("p"),
        Some("l"),
    )
    .unwrap();
    assert!(url.starts_with("https://h.example.com/"));
    let url_from_absolute_text = construct_target_url(
        "https://h.example.com",
        "https://evil.example.com/x",
        Some("p"),
        Some("l"),
    )
    .unwrap();
    assert!(url_from_absolute_text.starts_with("https://h.example.com/"));
}
