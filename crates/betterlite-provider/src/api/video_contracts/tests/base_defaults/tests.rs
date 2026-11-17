#![cfg(test)]

use rstest::rstest;

use crate::api::video_contracts::config::BaseVideoConfig as _;
use crate::api::video_contracts::records::{Headers, Params, VideoEnvironment};

use super::fixture::Subject;

#[rstest]
#[case::absent(None, Err("api_base is required"))]
#[case::empty(Some(""), Ok(""))]
#[case::given(Some("https://mock/videos"), Ok("https://mock/videos"))]
fn base_complete_url_needs_an_api_base(
    #[case] api_base: Option<&str>,
    #[case] expected: Result<&str, &str>,
) {
    let params = Params::new();
    let url = Subject
        .get_complete_url("model", api_base, &params)
        .map_err(|error| error.to_string());
    assert_eq!(url, expected.map(str::to_owned).map_err(str::to_owned));
}

#[test]
fn base_validate_environment_adds_no_headers() {
    let headers = Headers::from([("x-sent".to_owned(), "1".to_owned())]);
    let request = VideoEnvironment {
        headers: &headers,
        model: "model",
        api_key: None,
        litellm_params: None,
    };
    assert!(Subject.validate_environment(request).is_empty());
}
