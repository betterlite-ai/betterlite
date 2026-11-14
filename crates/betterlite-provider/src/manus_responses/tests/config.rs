use rstest::rstest;

use crate::api::responses_provider_contract::config::ResponsesProviderConfig as _;
use crate::api::responses_provider_contract::records::{Headers, Params};
use crate::compatible_providers::Compatible;
use crate::manus_responses::error::{MISSING_API_KEY_MESSAGE, ManusResponsesError};
use crate::manus_responses::request::agent_profile;
use crate::manus_responses::{
    API_KEY_HEADER, CONTENT_TYPE_HEADER, JSON_CONTENT_TYPE, MANUS_API_BASE_ENV, MANUS_API_KEY_ENV,
};
use crate::provider::Provider;

use super::support::{NOW, config, environment, globals, params, plain_config};

#[test]
fn the_provider_is_the_compatible_family() {
    assert_eq!(plain_config().provider(), Provider::Compatible(Compatible));
}

#[rstest]
#[case::unset(None, false)]
#[case::false_value(Some(false), false)]
#[case::true_value(Some(true), true)]
fn streaming_is_faked_exactly_when_stream_is_true(
    #[case] stream: Option<bool>,
    #[case] faked: bool,
) {
    assert_eq!(
        plain_config().should_fake_stream(Some("manus-1.6"), stream, Some("manus")),
        faked
    );
}

#[test]
fn native_websockets_are_off() {
    assert!(!plain_config().supports_native_websocket());
}

#[rstest]
#[case::prefixed("manus/manus-1.6", "manus-1.6")]
#[case::prefixed_lite("manus/manus-1.6-lite", "manus-1.6-lite")]
#[case::unprefixed("manus-1.6", "manus-1.6")]
#[case::two_slashes("a/b/c", "b/c")]
#[case::trailing_slash("manus/", "")]
fn the_agent_profile_is_the_text_after_the_first_slash(
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(agent_profile(model), expected);
}

#[rstest]
#[case::explicit_wins(Some("explicit"), Some("global"), &[(MANUS_API_KEY_ENV, "env")], "explicit")]
#[case::global_beats_the_environment(None, Some("global"), &[(MANUS_API_KEY_ENV, "env")], "global")]
#[case::environment_last(None, None, &[(MANUS_API_KEY_ENV, "env")], "env")]
#[case::empty_explicit_falls_through(Some(""), Some("global"), &[], "global")]
#[case::empty_global_falls_through(Some(""), Some(""), &[(MANUS_API_KEY_ENV, "env")], "env")]
fn the_key_follows_the_explicit_global_environment_chain(
    #[case] explicit: Option<&str>,
    #[case] global: Option<&str>,
    #[case] environment_pairs: &[(&str, &str)],
    #[case] expected: &str,
) {
    let litellm_params = match explicit {
        Some(key) => params(&format!(r#"{{"api_key":"{key}"}}"#)),
        None => Params::new(),
    };
    let settings = config(globals(None, global), environment(environment_pairs), NOW);
    let headers = settings
        .validate_environment(&Headers::new(), "", Some(&litellm_params))
        .expect("the fixture resolves a key");
    assert_eq!(
        headers.get(API_KEY_HEADER).map(String::as_str),
        Some(expected)
    );
    assert_eq!(
        headers.get(CONTENT_TYPE_HEADER).map(String::as_str),
        Some(JSON_CONTENT_TYPE)
    );
}

#[test]
fn a_missing_key_names_the_environment_variable() {
    let error = plain_config()
        .validate_environment(&Headers::new(), "", None)
        .expect_err("without a key there are no headers");
    assert_eq!(error.to_string(), MISSING_API_KEY_MESSAGE);
    assert!(matches!(error, ManusResponsesError::MissingApiKey));
}

#[test]
fn an_empty_environment_key_counts_as_unset() {
    let error = config(
        globals(None, None),
        environment(&[(MANUS_API_KEY_ENV, "")]),
        NOW,
    )
    .validate_environment(&Headers::new(), "", None)
    .expect_err("an empty environment value is unset");
    assert_eq!(error.to_string(), MISSING_API_KEY_MESSAGE);
}

#[test]
fn a_non_text_api_key_names_the_field() {
    let litellm_params = params(r#"{"api_key": 7}"#);
    let error = plain_config()
        .validate_environment(&Headers::new(), "", Some(&litellm_params))
        .expect_err("a number is not a key");
    assert_eq!(error.to_string(), "the api_key parameter must be text");
    assert!(matches!(
        error,
        ManusResponsesError::ParamNotText { field: "api_key" }
    ));
}

#[test]
fn headers_carry_the_api_key_and_json_content_type_beside_the_callers_headers() {
    let caller = Headers::from([(String::from("x-caller"), String::from("1"))]);
    let validated = config(globals(None, Some("global-key")), environment(&[]), NOW)
        .validate_environment(&caller, "", None)
        .expect("the global key resolves");
    assert_eq!(
        validated,
        Headers::from([
            (String::from(API_KEY_HEADER), String::from("global-key")),
            (
                String::from(CONTENT_TYPE_HEADER),
                String::from(JSON_CONTENT_TYPE)
            ),
            (String::from("x-caller"), String::from("1")),
        ])
    );
}

#[test]
fn the_api_key_headers_override_caller_values() {
    let caller = Headers::from([
        (String::from(API_KEY_HEADER), String::from("caller-key")),
        (
            String::from(CONTENT_TYPE_HEADER),
            String::from("text/plain"),
        ),
    ]);
    let validated = config(globals(None, Some("real-key")), environment(&[]), NOW)
        .validate_environment(&caller, "", None)
        .expect("the global key resolves");
    assert_eq!(
        validated.get(API_KEY_HEADER),
        Some(&String::from("real-key"))
    );
    assert_eq!(
        validated.get(CONTENT_TYPE_HEADER),
        Some(&String::from(JSON_CONTENT_TYPE))
    );
}

fn completed(
    api_base: Option<&str>,
    global: Option<&str>,
    environment_pairs: &[(&str, &str)],
) -> String {
    config(globals(global, None), environment(environment_pairs), NOW)
        .complete_url(api_base, &Params::new())
        .expect("the fixture builds a url")
}

#[rstest]
#[case::explicit(Some("https://explicit:1"), None, &[], "https://explicit:1/v1/responses")]
#[case::explicit_v1(Some("https://explicit:1/v1"), None, &[], "https://explicit:1/v1/responses")]
#[case::explicit_v1_slash(Some("https://explicit:1/v1/"), None, &[], "https://explicit:1/v1/responses")]
#[case::explicit_trailing_slashes(Some("https://explicit:1///"), None, &[], "https://explicit:1/v1/responses")]
#[case::global_base(None, Some("https://global:2"), &[], "https://global:2/v1/responses")]
#[case::environment_base(None, None, &[(MANUS_API_BASE_ENV, "https://env:3/v1")], "https://env:3/v1/responses")]
#[case::default_base(None, None, &[], "https://api.manus.im/v1/responses")]
#[case::empty_explicit(Some(""), Some("https://global:2"), &[(MANUS_API_BASE_ENV, "https://env:3")], "https://global:2/v1/responses")]
#[case::empty_global(None, Some(""), &[(MANUS_API_BASE_ENV, "https://env:3")], "https://env:3/v1/responses")]
#[case::empty_environment(None, None, &[(MANUS_API_BASE_ENV, "")], "https://api.manus.im/v1/responses")]
fn the_url_handles_the_default_base_a_trailing_slash_and_a_base_ending_in_v1(
    #[case] api_base: Option<&str>,
    #[case] global: Option<&str>,
    #[case] environment_pairs: &[(&str, &str)],
    #[case] expected: &str,
) {
    assert_eq!(completed(api_base, global, environment_pairs), expected);
}
