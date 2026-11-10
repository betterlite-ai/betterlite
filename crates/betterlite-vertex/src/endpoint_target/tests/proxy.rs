#![cfg(test)]

use rstest::rstest;

use crate::endpoint_target::error::EndpointTargetError;
use crate::endpoint_target::target::{
    AuthHeader, CustomProxyInput, TargetProvider, check_custom_proxy,
};
use crate::model_urls::ApiVersion;

use super::support::{
    LOCATION, MODEL, NATIVE_URL, PROJECT, header_of, secret, token_of, version_of,
};

#[rstest]
#[case::absent(
    None,
    None,
    NATIVE_URL,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::empty(
    Some(""),
    None,
    NATIVE_URL,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::host_only(
    Some("https://proxy.example.com"),
    None,
    NATIVE_URL,
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::host_only_slash(
    Some("https://proxy.example.com/"),
    None,
    NATIVE_URL,
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::host_only_with_query(
    Some("https://proxy.example.com?x=1"),
    None,
    NATIVE_URL,
    "https://proxy.example.com?x=1/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::version_root_v1(
    Some("https://proxy.example.com/v1"),
    None,
    NATIVE_URL,
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::version_root_v1beta1(
    Some("https://proxy.example.com/v1beta1"),
    None,
    NATIVE_URL,
    "https://proxy.example.com/v1beta1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::full_model(
    Some("https://proxy.example.com/v1/models/gemini-2.5-flash"),
    None,
    NATIVE_URL,
    "https://proxy.example.com/v1/models/gemini-2.5-flash:generateContent"
)]
#[case::full_model_slash(
    Some("https://proxy.example.com/foo/"),
    None,
    NATIVE_URL,
    "https://proxy.example.com/foo/:generateContent"
)]
#[case::version_root_without_projects(
    Some("https://proxy.example.com/v1"),
    None,
    "https://h.example.com/v1/foo:generateContent",
    "https://proxy.example.com/v1:generateContent"
)]
#[case::version_root_stream(
    Some("https://proxy.example.com/v1"),
    Some(true),
    NATIVE_URL,
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent?alt=sse"
)]
#[case::stream_with_query(
    Some("https://proxy.example.com/v1?x=1"),
    Some(true),
    NATIVE_URL,
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent?x=1&alt=sse"
)]
#[case::query_without_stream(
    Some("https://proxy.example.com/v1?x=1&y=2"),
    Some(false),
    NATIVE_URL,
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent?x=1&y=2"
)]
#[case::absent_base_ignores_stream(
    None,
    Some(true),
    NATIVE_URL,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
fn custom_proxy_cases(
    #[case] api_base: Option<&str>,
    #[case] stream: Option<bool>,
    #[case] url: &str,
    #[case] expected: &str,
) {
    let streaming = stream == Some(true);
    let input = CustomProxyInput {
        url,
        ..proxy_input!(api_base, streaming)
    };
    let target = check_custom_proxy(&input).unwrap();
    assert_eq!(target.url, expected);
    assert!(target.auth.is_none());
}

#[test]
fn custom_proxy_leaves_a_native_query_alone_without_a_custom_base() {
    let url = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse";
    let input = CustomProxyInput {
        url,
        stream: true,
        ..proxy_input!(None, false)
    };
    let target = check_custom_proxy(&input).unwrap();
    assert_eq!(target.url, url);
}

#[test]
fn custom_proxy_merges_a_full_model_base_query() {
    let input = CustomProxyInput {
        api_base: Some("https://proxy.example.com/foo?x=1"),
        ..proxy_input!(None, true)
    };
    let target = check_custom_proxy(&input).unwrap();
    assert_eq!(
        target.url,
        "https://proxy.example.com/foo?x=1:generateContent&alt=sse"
    );
}

#[test]
fn custom_proxy_appends_the_stream_query_before_an_existing_fragment() {
    let input = CustomProxyInput {
        api_base: Some("https://proxy.example.com/foo#frag"),
        url: "https://h.example.com/v1/foo:generateContent",
        ..proxy_input!(None, true)
    };
    assert_eq!(
        check_custom_proxy(&input).unwrap().url,
        "https://proxy.example.com/foo?alt=sse#frag:generateContent"
    );
}

#[test]
fn custom_proxy_passes_the_auth_token_through() {
    let auth = AuthHeader::Token(secret("tok"));
    let input = CustomProxyInput {
        auth_header: Some(&auth),
        ..proxy_input!(Some("https://proxy.example.com"), false)
    };
    let target = check_custom_proxy(&input).unwrap();
    assert_eq!(token_of(&target), Some("tok"));
}

#[rstest]
#[case::absent_version(
    Some("https://psc.example.com"),
    None,
    MODEL,
    "https://psc.example.com/v1/projects/proj/locations/us-central1/endpoints/gemini-2.5-flash:generateContent"
)]
#[case::default_version(
    Some("https://psc.example.com"),
    Some("v1"),
    MODEL,
    "https://psc.example.com/v1/projects/proj/locations/us-central1/endpoints/gemini-2.5-flash:generateContent"
)]
#[case::trailing_slash(
    Some("https://psc.example.com/"),
    Some("v1"),
    MODEL,
    "https://psc.example.com/v1/projects/proj/locations/us-central1/endpoints/gemini-2.5-flash:generateContent"
)]
#[case::explicit_v1beta1(
    Some("https://psc.example.com"),
    Some("v1beta1"),
    MODEL,
    "https://psc.example.com/v1beta1/projects/proj/locations/us-central1/endpoints/gemini-2.5-flash:generateContent"
)]
#[case::known_prefix(
    Some("https://psc.example.com"),
    Some("v1"),
    "bge/1234",
    "https://psc.example.com/v1/projects/proj/locations/us-central1/endpoints/1234:generateContent"
)]
#[case::unknown_prefix(
    Some("https://psc.example.com"),
    Some("v1"),
    "custom/publisher-model",
    "https://psc.example.com/v1/projects/proj/locations/us-central1/endpoints/custom/publisher-model:generateContent"
)]
fn psc_target_cases(
    #[case] api_base: Option<&str>,
    #[case] version: Option<&str>,
    #[case] model: &str,
    #[case] expected: &str,
) {
    let input = CustomProxyInput {
        api_base,
        model: Some(model),
        api_version: version.map_or(ApiVersion::V1, version_of),
        use_psc_endpoint_format: true,
        ..proxy_input!(None, false)
    };
    assert_eq!(check_custom_proxy(&input).unwrap().url, expected);
}

#[test]
fn psc_target_streams_with_alt_sse() {
    let input = CustomProxyInput {
        api_base: Some("https://psc.example.com"),
        stream: true,
        use_psc_endpoint_format: true,
        ..proxy_input!(None, false)
    };
    assert_eq!(
        check_custom_proxy(&input).unwrap().url,
        "https://psc.example.com/v1/projects/proj/locations/us-central1/endpoints/gemini-2.5-flash:generateContent?alt=sse"
    );
}

#[rstest]
#[case::missing_project(None, Some(LOCATION), Some(MODEL))]
#[case::missing_location(Some(PROJECT), None, Some(MODEL))]
#[case::missing_model(Some(PROJECT), Some(LOCATION), None)]
#[case::empty_project(Some(""), Some(LOCATION), Some(MODEL))]
#[case::empty_location(Some(PROJECT), Some(""), Some(MODEL))]
#[case::empty_model(Some(PROJECT), Some(LOCATION), Some(""))]
fn psc_target_requires_every_input(
    #[case] project: Option<&str>,
    #[case] location: Option<&str>,
    #[case] model: Option<&str>,
) {
    let input = CustomProxyInput {
        api_base: Some("https://psc.example.com"),
        project,
        location,
        model,
        use_psc_endpoint_format: true,
        ..proxy_input!(None, false)
    };
    assert!(matches!(
        check_custom_proxy(&input).unwrap_err(),
        EndpointTargetError::PscInputs
    ));
}

#[test]
fn gemini_custom_base_builds_the_model_path_and_key_header() {
    let key = secret("gk");
    let input = CustomProxyInput {
        api_base: Some("https://custom.example.com"),
        provider: TargetProvider::Gemini,
        gemini_api_key: Some(&key),
        ..proxy_input!(None, false)
    };
    let target = check_custom_proxy(&input).unwrap();
    assert_eq!(
        target.url,
        "https://custom.example.com/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(header_of(&target, "x-goog-api-key"), Some("gk"));
}

#[test]
fn gemini_custom_base_streams_with_alt_sse() {
    let key = secret("gk");
    let input = CustomProxyInput {
        api_base: Some("https://custom.example.com"),
        provider: TargetProvider::Gemini,
        gemini_api_key: Some(&key),
        endpoint: "streamGenerateContent",
        stream: true,
        ..proxy_input!(None, false)
    };
    assert_eq!(
        check_custom_proxy(&input).unwrap().url,
        "https://custom.example.com/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
    );
}

#[test]
fn gemini_custom_base_requires_a_model() {
    let key = secret("gk");
    let input = CustomProxyInput {
        api_base: Some("https://custom.example.com"),
        provider: TargetProvider::Gemini,
        gemini_api_key: Some(&key),
        model: None,
        ..proxy_input!(None, false)
    };
    assert!(matches!(
        check_custom_proxy(&input).unwrap_err(),
        EndpointTargetError::GeminiModel
    ));
}

#[test]
fn gemini_custom_base_requires_a_key() {
    let input = CustomProxyInput {
        api_base: Some("https://custom.example.com"),
        provider: TargetProvider::Gemini,
        ..proxy_input!(None, false)
    };
    assert!(matches!(
        check_custom_proxy(&input).unwrap_err(),
        EndpointTargetError::GeminiApiKey
    ));
}

#[test]
fn gemini_custom_base_wins_over_psc_format() {
    let key = secret("gk");
    let input = CustomProxyInput {
        api_base: Some("https://custom.example.com"),
        provider: TargetProvider::Gemini,
        gemini_api_key: Some(&key),
        use_psc_endpoint_format: true,
        ..proxy_input!(None, false)
    };
    assert_eq!(
        check_custom_proxy(&input).unwrap().url,
        "https://custom.example.com/models/gemini-2.5-flash:generateContent"
    );
}

#[test]
fn gemini_without_a_custom_base_returns_the_passed_auth() {
    let auth = AuthHeader::Token(secret("tok"));
    let input = CustomProxyInput {
        provider: TargetProvider::Gemini,
        auth_header: Some(&auth),
        ..proxy_input!(None, false)
    };
    let target = check_custom_proxy(&input).unwrap();
    assert_eq!(target.url, NATIVE_URL);
    assert_eq!(token_of(&target), Some("tok"));
}
