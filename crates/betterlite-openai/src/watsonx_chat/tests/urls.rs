use rstest::rstest;

use super::support::{auth_failure, environment, options};
use crate::watsonx_auth::error::WatsonxError;
use crate::watsonx_chat::error::WatsonxChatError;
use crate::watsonx_chat::urls::get_complete_url;

const HOST: &str = "https://host.example";

fn url_for(
    api_base: Option<&str>,
    model: &str,
    stream: bool,
    given: &str,
) -> (String, Vec<String>) {
    let mut found = options(given);
    let url = get_complete_url(
        api_base,
        model,
        stream,
        &mut found,
        &environment(None, None),
    )
    .unwrap();
    (url, found.into_keys().collect())
}

#[rstest]
#[case::plain("granite", false, "/ml/v1/text/chat")]
#[case::plain_stream("granite", true, "/ml/v1/text/chat_stream")]
#[case::deployment("deployment/abc", false, "/ml/v1/deployments/abc/text/chat")]
#[case::deployment_stream("deployment/abc", true, "/ml/v1/deployments/abc/text/chat_stream")]
#[case::deployment_keeps_slashes("deployment/a/b", false, "/ml/v1/deployments/a/b/text/chat")]
#[case::deployment_encodes_the_query(
    "deployment/x?y#z",
    true,
    "/ml/v1/deployments/x%3Fy%23z/text/chat_stream"
)]
#[case::prefix_needs_the_slash("deployment", false, "/ml/v1/text/chat")]
#[case::prefix_is_not_a_substring("my-deployment/abc", false, "/ml/v1/text/chat")]
#[case::provider_prefixed("ibm/granite", true, "/ml/v1/text/chat_stream")]
fn url_picks_the_endpoint(#[case] model: &str, #[case] stream: bool, #[case] path: &str) {
    let (url, _) = url_for(Some(HOST), model, stream, "{}");
    assert_eq!(url, format!("{HOST}{path}?version=2024-03-13"));
}

#[rstest]
#[case::one("https://host.example/")]
#[case::many("https://host.example////")]
#[case::none("https://host.example")]
fn url_strips_trailing_slashes_from_the_base(#[case] api_base: &str) {
    let (url, _) = url_for(Some(api_base), "granite", false, "{}");
    assert_eq!(
        url,
        "https://host.example/ml/v1/text/chat?version=2024-03-13"
    );
}

#[test]
fn url_keeps_a_path_in_the_base() {
    let (url, _) = url_for(Some("https://host.example/prefix/"), "granite", true, "{}");
    assert_eq!(
        url,
        "https://host.example/prefix/ml/v1/text/chat_stream?version=2024-03-13"
    );
}

#[test]
fn url_pops_the_api_version_and_uses_it() {
    let (url, left) = url_for(
        Some(HOST),
        "granite",
        false,
        r#"{"api_version": "2025-01-01", "temperature": 1}"#,
    );
    assert_eq!(url, format!("{HOST}/ml/v1/text/chat?version=2025-01-01"));
    assert_eq!(left, ["temperature"]);
}

#[rstest]
#[case::null(r#"{"api_version": null}"#)]
#[case::empty(r#"{"api_version": ""}"#)]
#[case::absent("{}")]
fn url_defaults_the_version(#[case] given: &str) {
    let (url, left) = url_for(Some(HOST), "granite", false, given);
    assert_eq!(url, format!("{HOST}/ml/v1/text/chat?version=2024-03-13"));
    assert!(left.is_empty());
}

#[rstest]
#[case::number(r#"{"api_version": 3}"#)]
#[case::boolean(r#"{"api_version": true}"#)]
#[case::list(r#"{"api_version": ["2025"]}"#)]
#[case::object(r#"{"api_version": {}}"#)]
fn url_rejects_a_version_that_is_not_text(#[case] given: &str) {
    let mut found = options(given);
    let error = get_complete_url(
        Some(HOST),
        "granite",
        false,
        &mut found,
        &environment(None, None),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        WatsonxChatError::InvalidField {
            field: "api_version"
        }
    ));
}

#[test]
fn url_refuses_a_deployment_id_that_leaves_its_path() {
    let mut found = options("{}");
    let error = get_complete_url(
        Some(HOST),
        "deployment/x/../other",
        false,
        &mut found,
        &environment(None, None),
    )
    .unwrap_err();
    assert!(matches!(
        auth_failure(&error),
        WatsonxError::Api {
            status_code: 400,
            ..
        }
    ));
}

#[test]
fn url_reads_the_environment_base() {
    let mut found = options("{}");
    let url = get_complete_url(
        None,
        "granite",
        false,
        &mut found,
        &environment(Some("https://env.example/"), None),
    )
    .unwrap();
    assert_eq!(
        url,
        "https://env.example/ml/v1/text/chat?version=2024-03-13"
    );
}

#[test]
fn url_without_any_base_fails_with_401_before_the_version_pop() {
    let mut found = options(r#"{"api_version": 3}"#);
    let error =
        get_complete_url(None, "granite", false, &mut found, &environment(None, None)).unwrap_err();
    assert!(matches!(
        auth_failure(&error),
        WatsonxError::Api {
            status_code: 401,
            ..
        }
    ));
    assert!(found.contains_key("api_version"));
}
