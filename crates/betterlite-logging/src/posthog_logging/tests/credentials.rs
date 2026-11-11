use betterlite_models::decisions::Json;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::posthog_logging::credentials::{CredentialError, credentials_for_request};
use crate::posthog_logging::tests::support::{
    capture_logs, default_settings, dynamic, object, text,
};

fn resolve(dynamic_params: &Json) -> Result<(String, String, bool), CredentialError> {
    let credentials = credentials_for_request(&default_settings(), dynamic_params)?;
    Ok((
        credentials.api_key.expose_secret().to_owned(),
        credentials.api_url,
        credentials.request_url,
    ))
}

#[test]
fn posthog_credentials_default_to_the_initialized_values() {
    assert_eq!(
        resolve(&Json::Null).expect("the credentials resolve"),
        (
            "phk-test".to_owned(),
            "https://us.i.posthog.com".to_owned(),
            false
        )
    );
}

#[test]
fn posthog_credentials_take_the_per_request_pair() {
    let params = dynamic(&[
        ("posthog_api_key", "request-key"),
        ("posthog_api_url", "https://request.example/"),
    ]);
    assert_eq!(
        resolve(&params).expect("the credentials resolve"),
        (
            "request-key".to_owned(),
            "https://request.example/".to_owned(),
            true
        )
    );
}

#[rstest]
#[case::key_only(
    &[("posthog_api_key", "request-key")],
    ("request-key", "https://us.i.posthog.com", false)
)]
#[case::url_only(
    &[("posthog_api_url", "https://request.example")],
    ("phk-test", "https://us.i.posthog.com", false)
)]
#[case::empty_key(
    &[("posthog_api_key", ""), ("posthog_api_url", "https://request.example")],
    ("phk-test", "https://us.i.posthog.com", false)
)]
#[case::empty_url(
    &[("posthog_api_key", "request-key"), ("posthog_api_url", "")],
    ("request-key", "https://us.i.posthog.com", false)
)]
fn posthog_credentials_fall_back_one_field_at_a_time(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: (&str, &str, bool),
) {
    let params = dynamic(pairs);
    assert_eq!(
        resolve(&params).expect("the credentials resolve"),
        (expected.0.to_owned(), expected.1.to_owned(), expected.2)
    );
}

#[test]
fn posthog_credentials_null_fields_use_the_defaults() {
    let params = object(&[
        ("posthog_api_key", Json::Null),
        ("posthog_api_url", Json::Null),
    ]);
    assert_eq!(
        resolve(&params).expect("the credentials resolve"),
        (
            "phk-test".to_owned(),
            "https://us.i.posthog.com".to_owned(),
            false
        )
    );
}

#[test]
fn posthog_credentials_reject_a_non_object_params_value() {
    let error = resolve(&text("request-key")).expect_err("the type is wrong");
    assert!(matches!(error, CredentialError::ParamsType));
}

#[rstest]
#[case::key("posthog_api_key")]
#[case::url("posthog_api_url")]
fn posthog_credentials_reject_a_non_text_field(#[case] field: &'static str) {
    let params = object(&[(field, Json::Bool(true))]);
    let error = resolve(&params).expect_err("the field type is wrong");
    assert!(matches!(
        error,
        CredentialError::FieldType { field: rejected } if rejected == field
    ));
}

#[rstest]
#[case::url_only(&[("posthog_api_url", "https://request.example")], true)]
#[case::empty_key(&[("posthog_api_key", ""), ("posthog_api_url", "https://request.example")], true)]
#[case::key_only(&[("posthog_api_key", "request-key")], false)]
#[case::both(&[("posthog_api_key", "request-key"), ("posthog_api_url", "https://request.example")], false)]
#[case::empty_url(&[("posthog_api_key", "request-key"), ("posthog_api_url", "")], false)]
fn posthog_credentials_warn_when_a_request_url_comes_without_a_key(
    #[case] pairs: &[(&str, &str)],
    #[case] warned: bool,
) {
    let (_guard, logs) = capture_logs();
    resolve(&dynamic(pairs)).expect("the credentials resolve");
    let output = logs.contents();
    assert_eq!(output.contains("ignored posthog_api_url"), warned);
    assert_eq!(output.contains("its own posthog_api_key"), warned);
}
