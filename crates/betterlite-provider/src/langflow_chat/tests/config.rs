use std::collections::BTreeMap;

use betterlite_secret_managers::secret_lookup::SecretLookup;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::support::{empty_object, environment, key, object, secrets};
use crate::langflow_chat::config::{
    API_BASE_ENV, API_KEY_ENV, DEFAULT_API_BASE, SUPPORTED_OPENAI_PARAMS, complete_url, flow_id,
    map_openai_params, provider_info, sign_request, validate_environment,
};
use crate::langflow_chat::error::{
    FLOW_ID_IN_PARAMS_MESSAGE, FLOW_ID_REQUIRED_MESSAGE, LangFlowError, TWEAKS_MESSAGE,
};

fn resolved(
    api_base: Option<&str>,
    api_key: Option<&str>,
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
) -> (String, Option<String>) {
    let secret = api_key.map(key);
    let info = provider_info(api_base, secret.as_ref(), secrets, environment)
        .expect("the fixture environment resolves");
    (
        info.api_base,
        info.api_key.map(|value| value.expose_secret().to_owned()),
    )
}

#[test]
fn supported_params_are_only_stream() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["stream"]);
}

#[test]
fn provider_info_prefers_the_explicit_base_and_key() {
    let environment = environment(&[(API_BASE_ENV, "http://env:7860"), (API_KEY_ENV, "env-key")]);
    let (base, api_key) = resolved(
        Some("http://explicit:9000"),
        Some("explicit-key"),
        &secrets(),
        &environment,
    );
    assert_eq!(base, "http://explicit:9000");
    assert_eq!(api_key.as_deref(), Some("explicit-key"));
}

#[test]
fn provider_info_reads_the_environment_variables() {
    let environment = environment(&[(API_BASE_ENV, "http://env:7860"), (API_KEY_ENV, "env-key")]);
    let (base, api_key) = resolved(None, None, &secrets(), &environment);
    assert_eq!(base, "http://env:7860");
    assert_eq!(api_key.as_deref(), Some("env-key"));
}

#[test]
fn provider_info_defaults_to_localhost_and_no_key() {
    let (base, api_key) = resolved(None, None, &secrets(), &BTreeMap::new());
    assert_eq!(base, DEFAULT_API_BASE);
    assert_eq!(api_key, None);
}

#[test]
fn provider_info_treats_empty_values_as_unset() {
    let environment = environment(&[(API_BASE_ENV, ""), (API_KEY_ENV, "")]);
    let (base, api_key) = resolved(Some(""), Some(""), &secrets(), &environment);
    assert_eq!(base, DEFAULT_API_BASE);
    assert_eq!(api_key, None);
}

#[test]
fn provider_info_keeps_the_key_out_of_debug_output() {
    let info = provider_info(
        Some("http://host:7860"),
        Some(&key("top-secret")),
        &secrets(),
        &BTreeMap::new(),
    )
    .expect("the fixture environment resolves");
    let printed = format!("{info:?}");
    assert!(!printed.contains("top-secret"));
}

#[test]
fn provider_info_drops_a_boolean_environment_value() {
    let environment = environment(&[(API_KEY_ENV, "true")]);
    let (_, api_key) = resolved(None, None, &secrets(), &environment);
    assert_eq!(api_key, None);
}

#[rstest]
#[case::provider_prefix("langflow/abc", "abc")]
#[case::whole_model("abc", "abc")]
#[case::first_slash_only("langflow/a/b", "a/b")]
#[case::leading_slash("/abc", "abc")]
#[case::surrounding_space("langflow/ abc\t", "abc")]
fn flow_id_comes_from_the_model(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(
        flow_id(model, &empty_object()).expect("the model names a flow"),
        expected
    );
}

#[test]
fn flow_id_in_the_request_parameters_is_refused() {
    let error = flow_id("langflow/abc", &object(r#"{"flow_id":"other"}"#))
        .expect_err("a request parameter cannot pick the flow");
    assert!(matches!(
        error,
        LangFlowError::Api { status_code: 400, message } if message == FLOW_ID_IN_PARAMS_MESSAGE
    ));
}

#[test]
fn a_null_flow_id_parameter_is_allowed() {
    assert_eq!(
        flow_id("langflow/abc", &object(r#"{"flow_id":null}"#)).expect("a null parameter is unset"),
        "abc"
    );
}

#[rstest]
#[case::prefix_only("langflow/")]
#[case::empty("")]
#[case::spaces("langflow/   ")]
fn empty_flow_ids_are_refused(#[case] model: &str) {
    let error = flow_id(model, &empty_object()).expect_err("an empty flow id cannot run");
    assert!(matches!(
        error,
        LangFlowError::Api { status_code: 400, message } if message == FLOW_ID_REQUIRED_MESSAGE
    ));
}

#[test]
fn complete_url_requires_a_base() {
    let error = complete_url(None, "langflow/abc", &empty_object())
        .expect_err("without a base there is no URL");
    assert!(matches!(error, LangFlowError::ApiBaseRequired));
    assert_eq!(error.status_code(), None);
}

#[rstest]
#[case::plain("http://host:7860", "http://host:7860/api/v1/run/abc")]
#[case::one_slash("http://host:7860/", "http://host:7860/api/v1/run/abc")]
#[case::many_slashes("http://host:7860///", "http://host:7860/api/v1/run/abc")]
fn complete_url_joins_the_run_path(#[case] base: &str, #[case] expected: &str) {
    assert_eq!(
        complete_url(Some(base), "langflow/abc", &empty_object())
            .expect("the fixture base builds a URL"),
        expected
    );
}

#[rstest]
#[case::space("langflow/a b", "a%20b")]
#[case::slash("langflow/a/b", "a%2Fb")]
#[case::question_and_fragment("langflow/a?b#c", "a%3Fb%23c")]
#[case::unreserved_kept("langflow/my.flow-1_2~3", "my.flow-1_2~3")]
#[case::non_ascii("langflow/\u{fc}", "%C3%BC")]
fn the_url_encodes_the_flow_id(#[case] model: &str, #[case] encoded: &str) {
    assert_eq!(
        complete_url(Some("http://host:7860"), model, &empty_object())
            .expect("the fixture flow id encodes"),
        format!("http://host:7860/api/v1/run/{encoded}")
    );
}

#[test]
fn the_url_refuses_a_dot_segment_flow_id() {
    let error = complete_url(Some("http://host:7860"), "langflow/..", &empty_object())
        .expect_err("a dot segment is not a path segment");
    assert!(matches!(
        &error,
        LangFlowError::FlowId { flow_id, .. } if flow_id == ".."
    ));
    assert_eq!(error.status_code(), None);
}

#[test]
fn map_openai_params_returns_the_options_unchanged() {
    let optional = object(r#"{"stream":true,"input_type":"text"}"#);
    assert_eq!(
        map_openai_params(
            &object(r#"{"temperature":0.2,"stream":true}"#),
            optional.clone(),
            "langflow/abc",
            false,
        ),
        optional
    );
}

#[test]
fn validate_environment_sets_the_json_content_type_and_the_key_header() {
    let mut headers = BTreeMap::new();
    validate_environment(&mut headers, Some(&key("secret")));
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(headers.get("x-api-key").map(String::as_str), Some("secret"));
    assert!(!headers.contains_key("Authorization"));
}

#[test]
fn validate_environment_overwrites_the_content_type() {
    let mut headers = BTreeMap::from([(String::from("Content-Type"), String::from("text/plain"))]);
    validate_environment(&mut headers, None);
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert!(!headers.contains_key("x-api-key"));
}

#[test]
fn validate_environment_skips_an_empty_key() {
    let mut headers = BTreeMap::new();
    validate_environment(&mut headers, Some(&key("")));
    assert!(!headers.contains_key("x-api-key"));
}

#[test]
fn sign_request_refuses_tweaks_in_the_request_body() {
    let error = sign_request(
        BTreeMap::new(),
        &object(r#"{"input_value":"hi","tweaks":{"x":{}}}"#),
    )
    .expect_err("a final body cannot carry tweaks");
    assert_eq!(error.status_code(), Some(400));
    assert!(matches!(
        &error,
        LangFlowError::Api { status_code: 400, message } if message.as_str() == TWEAKS_MESSAGE
    ));
}

#[test]
fn sign_request_returns_the_headers_and_no_body() {
    let headers = BTreeMap::from([(String::from("x-api-key"), String::from("secret"))]);
    assert_eq!(
        sign_request(
            headers.clone(),
            &object(r#"{"input_value":"hi","tweaks":null}"#)
        )
        .expect("a null tweaks value is unset"),
        headers
    );
}

#[test]
fn every_error_reports_its_status() {
    let api = LangFlowError::Api {
        status_code: 400,
        message: String::from("bad"),
    };
    assert_eq!(api.status_code(), Some(400));
    assert_eq!(LangFlowError::ApiBaseRequired.status_code(), None);
}
