use std::collections::BTreeMap;

use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;

use crate::api::file_contracts::config::{BaseFilesConfig as _, FileOpenAiParam, Headers};
use crate::manus_files::error::{MISSING_API_KEY_MESSAGE, ManusFilesError};
use crate::manus_files::{
    API_KEY_HEADER, CONTENT_TYPE_HEADER, JSON_CONTENT_TYPE, MANUS_API_KEY_ENV, ManusFilesConfig,
};

use super::support::{NOW, config, environment, globals, key, plain_config};

fn validated(
    explicit: Option<&str>,
    global: Option<&str>,
    environment_pairs: &[(&str, &str)],
) -> Headers {
    let explicit_key = explicit.map(key);
    let mut headers = Headers::new();
    config(globals(None, global), environment(environment_pairs), NOW)
        .validate_environment(&mut headers, explicit_key.as_ref())
        .expect("the fixture resolves a key");
    headers
}

#[test]
fn the_provider_is_manus() {
    assert_eq!(plain_config(NOW).custom_llm_provider(), ProviderId::Manus);
}

#[test]
fn supported_params_are_only_purpose() {
    assert_eq!(
        plain_config(NOW).get_supported_openai_params("manus-1.6"),
        vec![FileOpenAiParam::Purpose]
    );
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
    assert_eq!(
        validated(explicit, global, environment_pairs).get(API_KEY_HEADER),
        Some(&String::from(expected))
    );
}

#[test]
fn a_missing_key_names_the_environment_variable() {
    let mut headers = Headers::new();
    let error = plain_config(NOW)
        .validate_environment(&mut headers, None)
        .expect_err("without a key there are no headers");
    assert_eq!(error.to_string(), MISSING_API_KEY_MESSAGE);
    assert!(matches!(error, ManusFilesError::MissingApiKey));
}

#[test]
fn an_empty_environment_key_counts_as_unset() {
    let mut headers = Headers::new();
    let error = config(
        globals(None, None),
        environment(&[(MANUS_API_KEY_ENV, "")]),
        NOW,
    )
    .validate_environment(&mut headers, None)
    .expect_err("an empty environment value is unset");
    assert_eq!(error.to_string(), MISSING_API_KEY_MESSAGE);
}

#[test]
fn headers_carry_the_api_key_and_json_content_type_beside_the_callers_headers() {
    let mut headers = Headers::from([(String::from("x-caller"), String::from("1"))]);
    config(globals(None, Some("global-key")), environment(&[]), NOW)
        .validate_environment(&mut headers, None)
        .expect("the global key resolves");
    assert_eq!(
        headers,
        BTreeMap::from([
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
fn get_error_class_carries_the_status_message_and_headers() {
    let headers = Headers::from([(String::from("x"), String::from("y"))]);
    let error = ManusFilesConfig::get_error_class("boom", 418, headers.clone());
    assert_eq!(error.status_code, 418);
    assert_eq!(error.message, "boom");
    assert_eq!(error.headers, headers);
    assert_eq!(error.to_string(), "boom");
}
