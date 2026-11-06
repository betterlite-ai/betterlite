use std::borrow::Cow;
use std::collections::BTreeMap;

use http::header::CONTENT_TYPE;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;

use super::super::constants::{API_BASE_VARIABLE, API_KEY_VARIABLE};
use super::super::request::{SandboxRef, as_str_dict, endpoint_base_url, lifecycle_headers};
use super::{FakeHttp, environment, handle, sandbox};

#[test]
fn validate_environment_prefers_the_supplied_key() {
    let sandbox = sandbox(
        FakeHttp::new(&[]),
        environment(&[(API_KEY_VARIABLE, "env-key")]),
    );
    let supplied = SecretString::from("supplied-key");
    assert_eq!(
        sandbox
            .validate_environment(Some(&supplied))
            .expose_secret(),
        "supplied-key"
    );
}

#[test]
fn validate_environment_reads_the_open_sandbox_api_key() {
    let sandbox = sandbox(
        FakeHttp::new(&[]),
        environment(&[(API_KEY_VARIABLE, "env-key")]),
    );
    assert_eq!(
        sandbox.validate_environment(None).expose_secret(),
        "env-key"
    );
}

#[test]
fn validate_environment_treats_an_empty_variable_as_unset() {
    let sandbox = sandbox(FakeHttp::new(&[]), environment(&[(API_KEY_VARIABLE, "")]));
    assert_eq!(sandbox.validate_environment(None).expose_secret(), "");
}

#[test]
fn api_base_requires_a_base_and_names_the_variable() {
    let sandbox = sandbox(FakeHttp::new(&[]), environment(&[]));
    let error = sandbox.api_base(None).unwrap_err();
    assert!(error.to_string().contains(API_BASE_VARIABLE));
}

#[test]
fn api_base_reads_the_environment_and_trims_trailing_slashes() {
    let sandbox = sandbox(
        FakeHttp::new(&[]),
        environment(&[(API_BASE_VARIABLE, "http://sandbox.example///")]),
    );
    assert_eq!(sandbox.api_base(None).unwrap(), "http://sandbox.example");
}

#[test]
fn api_base_prefers_the_parameter_over_the_environment() {
    let sandbox = sandbox(
        FakeHttp::new(&[]),
        environment(&[(API_BASE_VARIABLE, "http://env.example")]),
    );
    assert_eq!(
        sandbox.api_base(Some("https://explicit.example/")).unwrap(),
        "https://explicit.example"
    );
}

#[test]
fn api_base_rejects_an_explicit_empty_value_even_with_the_environment_set() {
    let sandbox = sandbox(
        FakeHttp::new(&[]),
        environment(&[(API_BASE_VARIABLE, "http://env.example")]),
    );
    assert!(matches!(
        sandbox.api_base(Some("")).unwrap_err(),
        super::super::OpenSandboxError::MissingApiBase { .. }
    ));
}

#[test]
fn lifecycle_headers_carry_json_content_type_without_a_key() {
    let headers = lifecycle_headers(&SecretString::from("")).unwrap();
    assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
    assert_eq!(headers.len(), 1);
}

#[test]
fn lifecycle_headers_carry_a_sensitive_api_key_header() {
    let headers = lifecycle_headers(&SecretString::from("secret-key")).unwrap();
    assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
    let key = headers.get("open-sandbox-api-key").unwrap();
    assert_eq!(key, "secret-key");
    assert!(key.is_sensitive());
}

#[rstest]
#[case::qualified_http(
    "http://10.0.0.5:44772/",
    "https://api.example",
    "http://10.0.0.5:44772"
)]
#[case::qualified_https(
    "https://10.0.0.5:44772",
    "http://api.example",
    "https://10.0.0.5:44772"
)]
#[case::relative_from_https("10.0.0.5:44772", "https://api.example", "https://10.0.0.5:44772")]
#[case::relative_from_http("10.0.0.5:44772/", "http://api.example", "http://10.0.0.5:44772")]
#[case::relative_without_protocol("10.0.0.5:44772", "api.example", "http://10.0.0.5:44772")]
fn endpoint_base_url_normalizes_the_endpoint(
    #[case] endpoint: &str,
    #[case] api_base: &str,
    #[case] expected: &str,
) {
    assert_eq!(endpoint_base_url(endpoint, api_base), expected);
}

#[test]
fn as_handle_returns_a_supplied_handle_unchanged() {
    let sandbox = sandbox(FakeHttp::new(&[]), environment(&[]));
    let container = handle(None);
    let resolved = sandbox
        .as_handle(SandboxRef::Handle(&container), None)
        .unwrap();
    assert!(matches!(resolved, Cow::Borrowed(_)));
    assert_eq!(resolved.id, "sbx_1");
}

#[test]
fn as_handle_builds_a_handle_from_an_id_and_the_api_base() {
    let sandbox = sandbox(FakeHttp::new(&[]), environment(&[]));
    let resolved = sandbox
        .as_handle(SandboxRef::Id("sbx_9"), Some("http://sandbox.example/"))
        .unwrap();
    assert_eq!(resolved.id, "sbx_9");
    assert_eq!(resolved.provider, "opensandbox");
    assert_eq!(resolved.domain.as_deref(), Some("http://sandbox.example"));
    assert!(resolved.hidden_params.api_key.is_none());
    assert!(resolved.hidden_params.api_base.is_none());
}

#[test]
fn as_handle_needs_a_base_for_an_id() {
    let sandbox = sandbox(FakeHttp::new(&[]), environment(&[]));
    assert!(matches!(
        sandbox
            .as_handle(SandboxRef::Id("sbx_9"), None)
            .unwrap_err(),
        super::super::OpenSandboxError::MissingApiBase { .. }
    ));
}

#[rstest]
#[case::parameter_wins(Some("parameter-key"), Some("stored-key"), "parameter-key")]
#[case::stored_key_is_next(None, Some("stored-key"), "stored-key")]
#[case::environment_is_last(None, None, "env-key")]
fn api_key_resolves_the_documented_precedence(
    #[case] parameter: Option<&str>,
    #[case] stored: Option<&str>,
    #[case] expected: &str,
) {
    let sandbox = sandbox(
        FakeHttp::new(&[]),
        environment(&[(API_KEY_VARIABLE, "env-key")]),
    );
    let container = handle(stored);
    let supplied = parameter.map(SecretString::from);
    assert_eq!(
        sandbox
            .api_key(supplied.as_ref(), &container)
            .expose_secret(),
        expected
    );
}

#[test]
fn as_str_dict_stringifies_every_value() {
    let fields: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(r#"{"a":"b","n":5,"z":null}"#).unwrap();
    assert_eq!(
        as_str_dict(Some(&fields)),
        BTreeMap::from([
            ("a".to_owned(), "b".to_owned()),
            ("n".to_owned(), "5".to_owned()),
            ("z".to_owned(), "null".to_owned()),
        ])
    );
}

#[test]
fn as_str_dict_is_empty_without_a_mapping() {
    assert!(as_str_dict(None).is_empty());
}
