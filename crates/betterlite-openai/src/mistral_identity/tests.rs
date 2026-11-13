#![cfg(test)]

use betterlite_config::sections::mistral_resources::Settings;
use rstest::rstest;
use secrecy::SecretString;

use super::error::MistralIdentityError;
use super::{DEFAULT_API_BASE, auth_headers, base_url};
use crate::openai_base_config::Headers;

fn settings(api_base: Option<&str>, api_key: Option<&str>) -> Settings {
    Settings {
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(SecretString::from),
    }
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::default(None, None, DEFAULT_API_BASE)]
#[case::environment(None, Some("https://env.example"), "https://env.example")]
#[case::explicit(
    Some("https://explicit.example"),
    Some("https://env.example"),
    "https://explicit.example"
)]
#[case::trailing_slashes(Some("https://explicit.example///"), None, "https://explicit.example")]
#[case::one_v1(Some("https://explicit.example/v1"), None, "https://explicit.example")]
#[case::v1_and_slash(Some("https://explicit.example/v1/"), None, "https://explicit.example")]
#[case::two_v1(
    Some("https://explicit.example/v1/v1"),
    None,
    "https://explicit.example/v1"
)]
#[case::other_version(
    Some("https://explicit.example/v2"),
    None,
    "https://explicit.example/v2"
)]
#[case::only_slashes(Some("/"), None, "")]
#[case::explicit_empty(Some(""), Some("https://env.example"), "")]
fn resolves_the_resource_origin(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(base_url(explicit, &settings(environment, None)), expected);
}

#[test]
fn the_default_origin_has_no_version() {
    assert_eq!(DEFAULT_API_BASE, "https://api.mistral.ai");
}

#[rstest]
#[case::explicit_key(
    Some("explicit-key"),
    None,
    &[("X-Trace", "1")],
    &[("X-Trace", "1"), ("Authorization", "Bearer explicit-key")]
)]
#[case::environment_key(
    None,
    Some("environment-key"),
    &[],
    &[("Authorization", "Bearer environment-key")]
)]
#[case::explicit_beats_environment(
    Some("explicit-key"),
    Some("environment-key"),
    &[],
    &[("Authorization", "Bearer explicit-key")]
)]
#[case::replaces_authorization(
    Some("explicit-key"),
    None,
    &[("Authorization", "Bearer old")],
    &[("Authorization", "Bearer explicit-key")]
)]
#[case::keeps_a_lowercase_authorization(
    Some("explicit-key"),
    None,
    &[("authorization", "Bearer old")],
    &[("authorization", "Bearer old"), ("Authorization", "Bearer explicit-key")]
)]
fn replaces_the_resource_authorization(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] given: &[(&str, &str)],
    #[case] expected: &[(&str, &str)],
) {
    let given_key = explicit.map(SecretString::from);
    let key = given_key.as_ref();
    let resolved =
        auth_headers(&headers(given), key, &settings(None, environment)).expect("a key resolves");
    assert_eq!(resolved, headers(expected));
}

#[test]
fn a_resource_call_without_a_key_fails() {
    let error = auth_headers(&headers(&[("X-Trace", "1")]), None, &settings(None, None))
        .expect_err("no key fails");
    assert!(matches!(error, MistralIdentityError::MissingApiKey));
    assert!(error.to_string().contains("MISTRAL_API_KEY"));
}

#[test]
fn an_api_failure_keeps_its_status_message_and_headers() {
    let error = MistralIdentityError::Api {
        status: 429,
        message: String::from("slow down"),
        headers: headers(&[("retry-after", "2")]),
    };
    let MistralIdentityError::Api {
        status,
        message,
        headers: kept,
    } = &error
    else {
        panic!("the error is an API failure");
    };
    assert_eq!(*status, 429);
    assert_eq!(message, "slow down");
    assert_eq!(kept.get("retry-after").map(String::as_str), Some("2"));
    assert_eq!(error.to_string(), "the Mistral API returned 429: slow down");
}
