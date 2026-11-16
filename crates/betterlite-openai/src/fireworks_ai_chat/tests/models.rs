use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::super::auth::FireworksChatEnv;
use super::super::error::FireworksChatError;
use super::super::models::{MAX_RESPONSE_BYTES, decode_models_response, get_models_request};
use super::{env_keys, models_env, secret};

#[test]
fn models_request_builds_the_default_account_url() {
    let request = get_models_request(None, Some(&secret("key")), &models_env(None, Some("acct")))
        .expect("request");
    assert_eq!(
        request.url,
        "https://api.fireworks.ai/inference/v1/accounts/acct/models"
    );
    assert_eq!(
        request.headers.get("Authorization").map(String::as_str),
        Some("Bearer key")
    );
    assert_eq!(request.headers.len(), 1);
}

#[rstest]
#[case::versioned("https://custom/v1", "https://custom/v1/accounts/acct/models")]
#[case::versioned_with_slash("https://custom/v1/", "https://custom/v1/accounts/acct/models")]
#[case::bare_host("https://custom/", "https://custom/v1/accounts/acct/models")]
#[case::doubled_version("https://custom/v1/v1", "https://custom/v1/v1/accounts/acct/models")]
fn models_request_trims_the_base(#[case] api_base: &str, #[case] expected: &str) {
    let request = get_models_request(
        Some(api_base),
        Some(&secret("key")),
        &models_env(None, Some("acct")),
    )
    .expect("request");
    assert_eq!(request.url, expected);
}

#[test]
fn models_request_uses_the_environment_base_and_key() {
    let environment = FireworksChatEnv {
        keys: env_keys(Some("env-key"), None, None, None),
        api_base: Some(String::from("https://env/v1")),
        account_id: Some(String::from("acct")),
    };
    let request = get_models_request(None, None, &environment).expect("request");
    assert_eq!(request.url, "https://env/v1/accounts/acct/models");
    assert_eq!(
        request.headers.get("Authorization").map(String::as_str),
        Some("Bearer env-key")
    );
}

#[test]
fn models_request_keeps_a_supplied_key_over_the_environment() {
    let environment = FireworksChatEnv {
        keys: env_keys(Some("env-key"), None, None, None),
        api_base: Some(String::from("https://env/v1")),
        account_id: Some(String::from("acct")),
    };
    let request =
        get_models_request(None, Some(&secret("argument")), &environment).expect("request");
    assert_eq!(
        request.headers.get("Authorization").map(String::as_str),
        Some("Bearer argument")
    );
}

#[test]
fn models_request_without_a_key_names_both_variables() {
    let error =
        get_models_request(None, None, &models_env(None, Some("acct"))).expect_err("no key");
    assert!(
        error
            .to_string()
            .contains("FIREWORKS_API_BASE or FIREWORKS_API_KEY")
    );
    assert_eq!(error.status_code(), None);
}

#[test]
fn models_request_without_an_account_names_it() {
    let error = get_models_request(None, Some(&secret("key")), &models_env(None, None))
        .expect_err("no account");
    assert!(error.to_string().contains("FIREWORKS_ACCOUNT_ID"));
    assert_eq!(error.status_code(), None);
}

#[test]
fn models_request_reads_an_empty_account_as_unset() {
    let error = get_models_request(None, Some(&secret("key")), &models_env(None, Some("")))
        .expect_err("no account");
    assert!(error.to_string().contains("FIREWORKS_ACCOUNT_ID"));
}

#[test]
fn decode_models_response_prefixes_each_name_in_order() {
    let body = br#"{"models":[{"name":"llama-v3"},{"name":"qwen3"}]}"#;
    let models = decode_models_response(200, body).expect("models");
    assert_eq!(
        models,
        vec![
            String::from("fireworks_ai/llama-v3"),
            String::from("fireworks_ai/qwen3")
        ]
    );
}

#[test]
fn decode_models_response_reports_the_status_and_body() {
    let error = decode_models_response(503, br#"{"error":"down"}"#).expect_err("status");
    let message = error.to_string();
    assert!(message.contains("Status code: 503"));
    assert!(message.contains(r#"{"error":"down"}"#));
    assert_eq!(error.status_code(), None);
}

#[test]
fn decode_models_response_rejects_a_reply_that_is_not_json() {
    let error = decode_models_response(200, b"nope").expect_err("decode");
    assert_eq!(error.status_code(), None);
    assert!(error.to_string().contains("not valid JSON"));
}

#[test]
fn decode_models_response_rejects_a_reply_without_a_models_list() {
    let error = decode_models_response(200, br#"{"data":[]}"#).expect_err("shape");
    assert!(error.to_string().contains("models"));
}

#[test]
fn the_models_response_limit_is_one_mib() {
    assert_eq!(MAX_RESPONSE_BYTES, 1024 * 1024);
}

#[test]
fn decode_models_response_reads_a_body_of_exactly_the_limit() {
    let mut bytes = vec![b' '; MAX_RESPONSE_BYTES];
    bytes[..15].copy_from_slice(br#"{"models":[]}  "#);
    let models = decode_models_response(200, &bytes).expect("limit");
    assert!(models.is_empty());
}

#[test]
fn decode_models_response_rejects_a_body_over_the_limit() {
    let bytes = vec![b' '; MAX_RESPONSE_BYTES + 1];
    let error = decode_models_response(200, &bytes).expect_err("over the limit");
    assert!(matches!(error, FireworksChatError::ModelsTooLarge));
}

#[rstest]
fn replays_the_models_response_corpus(
    #[files("fuzz/corpus/fireworks_ai_models_response/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    let _decoded = decode_models_response(200, &bytes);
}
