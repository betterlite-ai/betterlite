use betterlite_models::decisions::Json;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::posthog_logging::credentials::{CredentialError, credentials_for_request};
use crate::posthog_logging::tests::support::{default_settings, dynamic, object, text};

fn resolve(dynamic_params: &Json) -> Result<(String, String), CredentialError> {
    let credentials = credentials_for_request(&default_settings(), dynamic_params)?;
    Ok((
        credentials.api_key.expose_secret().to_owned(),
        credentials.api_url,
    ))
}

#[test]
fn posthog_credentials_default_to_the_initialized_values() {
    assert_eq!(
        resolve(&Json::Null).expect("the credentials resolve"),
        ("phk-test".to_owned(), "https://us.i.posthog.com".to_owned())
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
            "https://request.example/".to_owned()
        )
    );
}

#[rstest]
#[case::key_only(
    &[("posthog_api_key", "request-key")],
    ("request-key", "https://us.i.posthog.com")
)]
#[case::url_only(
    &[("posthog_api_url", "https://request.example")],
    ("phk-test", "https://request.example")
)]
#[case::empty_key(
    &[("posthog_api_key", ""), ("posthog_api_url", "https://request.example")],
    ("phk-test", "https://request.example")
)]
#[case::empty_url(
    &[("posthog_api_key", "request-key"), ("posthog_api_url", "")],
    ("request-key", "https://us.i.posthog.com")
)]
fn posthog_credentials_fall_back_one_field_at_a_time(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: (&str, &str),
) {
    let params = dynamic(pairs);
    assert_eq!(
        resolve(&params).expect("the credentials resolve"),
        (expected.0.to_owned(), expected.1.to_owned())
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
        ("phk-test".to_owned(), "https://us.i.posthog.com".to_owned())
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
