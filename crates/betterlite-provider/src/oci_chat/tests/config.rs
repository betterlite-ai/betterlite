use betterlite_openai::chat::Role;
use http::HeaderMap;
use rstest::rstest;
use secrecy::SecretString;

use crate::oci_chat::config::{
    get_complete_url, get_error_class, get_supported_openai_params, get_vendor_from_model,
    map_openai_params, uses_max_completion_tokens, validate_environment,
};
use crate::oci_chat::tests::support::{FakePredicates, assert_json, credentials, message, params};
use crate::oci_chat_wire::types::oci::OciVendor;
use crate::oci_common::environment::OciEnvironment;

const LLAMA: &str = "meta.llama-3.3-70b-instruct";

#[test]
fn supported_params_lists_each_vendors_map() {
    assert_eq!(
        get_supported_openai_params(LLAMA),
        vec![
            "stream",
            "max_tokens",
            "max_completion_tokens",
            "temperature",
            "tools",
            "frequency_penalty",
            "logprobs",
            "logit_bias",
            "n",
            "presence_penalty",
            "reasoning_effort",
            "seed",
            "stop",
            "tool_choice",
            "top_p",
            "response_format",
        ]
    );
    assert_eq!(
        get_supported_openai_params("cohere.command-r"),
        vec![
            "stream",
            "max_tokens",
            "max_completion_tokens",
            "temperature",
            "tools",
            "frequency_penalty",
            "presence_penalty",
            "seed",
            "stop",
            "top_p",
            "response_format",
            "top_k",
        ]
    );
}

#[rstest]
#[case("cohere.command-r", OciVendor::Cohere)]
#[case("oci/cohere.command-r", OciVendor::Cohere)]
#[case("OCI/Cohere.command-r", OciVendor::Cohere)]
#[case(LLAMA, OciVendor::Generic)]
#[case("oci/openai.gpt-5", OciVendor::Generic)]
#[case("", OciVendor::Generic)]
fn vendor_from_model_reads_the_prefix_and_first_segment(
    #[case] model: &str,
    #[case] vendor: OciVendor,
) {
    assert_eq!(get_vendor_from_model(model), vendor);
}

#[test]
fn map_params_renames_and_keeps_passthrough_keys() {
    let adapted = map_openai_params(
        &params(
            r#"{"max_tokens": 5, "stream": true, "response_format": {"type": "text"}, "user": "abc"}"#,
        ),
        &params("{}"),
        LLAMA,
        false,
    )
    .unwrap();
    assert_json(
        &adapted,
        r#"{"maxTokens": 5, "isStream": true, "responseFormat": {"type": "text"}, "response_format": {"type": "text"}, "user": "abc"}"#,
    );
}

#[test]
fn map_params_lets_optional_params_win() {
    let adapted = map_openai_params(
        &params(r#"{"temperature": 0.1, "seed": 1}"#),
        &params(r#"{"temperature": 0.9}"#),
        LLAMA,
        false,
    )
    .unwrap();
    assert_json(&adapted, r#"{"temperature": 0.9, "seed": 1}"#);
}

#[test]
fn map_params_rejects_unsupported_params() {
    let refusal = map_openai_params(
        &params("{}"),
        &params(r#"{"modalities": ["text"]}"#),
        LLAMA,
        false,
    )
    .unwrap_err();
    assert_eq!(refusal.status, 400);
    assert_eq!(
        refusal.message,
        "param `modalities` is not supported on OCI"
    );
}

#[test]
fn map_params_drops_unsupported_params_when_asked() {
    let dropped = map_openai_params(
        &params(r#"{"modalities": ["text"], "top_logprobs": 2}"#),
        &params("{}"),
        LLAMA,
        true,
    )
    .unwrap();
    assert!(dropped.is_empty());
}

#[test]
fn map_params_skips_max_retries() {
    let adapted = map_openai_params(
        &params(r#"{"max_retries": 3, "top_p": 0.5}"#),
        &params("{}"),
        LLAMA,
        false,
    )
    .unwrap();
    assert_json(&adapted, r#"{"topP": 0.5}"#);
}

#[rstest]
#[case::int_one("1", false)]
#[case::float_one("1.0", false)]
#[case::null("null", false)]
#[case::two("2", true)]
#[case::fraction("1.5", true)]
#[case::text(r#""1""#, true)]
#[case::boolean("true", true)]
fn map_params_skips_n_equal_to_one_on_cohere(#[case] given: &str, #[case] rejected: bool) {
    let outcome = map_openai_params(
        &params(&format!(r#"{{"n": {given}, "top_p": 0.5}}"#)),
        &params("{}"),
        "cohere.command-r",
        false,
    );
    assert_eq!(outcome.is_err(), rejected);
    if rejected {
        let refusal = outcome.unwrap_err();
        assert_eq!(refusal.status, 400);
        assert_eq!(refusal.message, "param `n` is not supported on OCI");
    } else {
        assert_json(&outcome.unwrap(), r#"{"topP": 0.5}"#);
    }
}

#[test]
fn map_params_applies_the_cohere_map() {
    let adapted = map_openai_params(
        &params(r#"{"stop": ["x"], "top_k": 4}"#),
        &params("{}"),
        "cohere.command-r",
        false,
    )
    .unwrap();
    assert_json(&adapted, r#"{"stopSequences": ["x"], "topK": 4}"#);
}

#[test]
fn map_params_rejects_cohere_logit_bias() {
    let refusal = map_openai_params(
        &params(r#"{"logit_bias": {"1": 2}}"#),
        &params("{}"),
        "cohere.command-r",
        false,
    )
    .unwrap_err();
    assert_eq!(
        refusal.message,
        "param `logit_bias` is not supported on OCI"
    );
}

#[test]
fn validate_environment_requires_messages() {
    let refusal = validate_environment(
        &mut HeaderMap::new(),
        &[],
        &credentials(),
        &OciEnvironment::default(),
    )
    .unwrap_err();
    assert_eq!(refusal.status, 400);
    assert_eq!(
        refusal.message,
        "kwarg `messages` must be an array of messages that follow the openai chat standard"
    );
}

#[rstest]
#[case::user("oci_user")]
#[case::fingerprint("oci_fingerprint")]
#[case::tenancy("oci_tenancy")]
#[case::compartment("oci_compartment_id")]
fn validate_environment_requires_every_identity_field(#[case] field: &str) {
    let mut incomplete = credentials();
    match field {
        "oci_user" => incomplete.oci_user = None,
        "oci_fingerprint" => incomplete.oci_fingerprint = None,
        "oci_tenancy" => incomplete.oci_tenancy = None,
        _ => incomplete.oci_compartment_id = Some(String::new()),
    }
    let refusal = validate_environment(
        &mut HeaderMap::new(),
        &[message(Role::User, "hi")],
        &incomplete,
        &OciEnvironment::default(),
    )
    .unwrap_err();
    assert_eq!(refusal.status, 401);
    assert!(
        refusal
            .message
            .starts_with("Missing required parameters: oci_user, oci_fingerprint")
    );
}

#[rstest]
#[case::key_only(Some("key"), None, None)]
#[case::key_file_only(None, Some("/keys/oci.pem"), None)]
#[case::empty_key_with_file(Some(""), Some("/keys/oci.pem"), None)]
#[case::key_with_empty_file(Some("key"), Some(""), None)]
#[case::neither(None, None, Some(401_u16))]
#[case::empty_key(Some(""), None, Some(401_u16))]
#[case::empty_file(None, Some(""), Some(401_u16))]
#[case::both_empty(Some(""), Some(""), Some(401_u16))]
fn validate_environment_needs_a_key_or_a_key_file(
    #[case] key: Option<&str>,
    #[case] key_file: Option<&str>,
    #[case] refused_with: Option<u16>,
) {
    let mut keyed = credentials();
    keyed.oci_key = key.map(SecretString::from);
    keyed.oci_key_file = key_file.map(str::to_owned);
    let outcome = validate_environment(
        &mut HeaderMap::new(),
        &[message(Role::User, "hi")],
        &keyed,
        &OciEnvironment::default(),
    );
    assert_eq!(outcome.err().map(|refusal| refusal.status), refused_with);
}

#[test]
fn validate_environment_adds_the_json_headers() {
    let mut headers = HeaderMap::new();
    validate_environment(
        &mut headers,
        &[message(Role::User, "hi")],
        &credentials(),
        &OciEnvironment::default(),
    )
    .unwrap();
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(headers["user-agent"], "litellm/1.105.1");
}

#[test]
fn validate_environment_accepts_a_key_file_without_a_key() {
    let mut key_file_only = credentials();
    key_file_only.oci_key = None;
    key_file_only.oci_key_file = Some(String::from("/keys/oci.pem"));
    let mut headers = HeaderMap::new();
    validate_environment(
        &mut headers,
        &[message(Role::User, "hi")],
        &key_file_only,
        &OciEnvironment::default(),
    )
    .unwrap();
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(headers["user-agent"], "litellm/1.105.1");
}

#[test]
fn complete_url_uses_the_chat_action() {
    assert_eq!(
        get_complete_url(&credentials(), None).unwrap(),
        "https://inference.generativeai.us-ashburn-1.oci.oraclecloud.com/20231130/actions/chat"
    );
    assert_eq!(
        get_complete_url(&credentials(), Some("https://proxy.example")).unwrap(),
        "https://proxy.example/20231130/actions/chat"
    );
}

#[test]
fn get_error_class_keeps_status_and_message() {
    let error = get_error_class("boom", 418);
    assert_eq!(error.status, 418);
    assert_eq!(error.message, "boom");
}

#[rstest]
#[case::reasoning_gemini("google.gemini-2.5-flash", true, true)]
#[case::plain_gemini("google.gemini-2.5-flash", false, false)]
#[case::openai("openai.gpt-5", false, true)]
#[case::gpt_oss("OCI/openai.gpt-oss-120b", false, false)]
#[case::gpt_oss_reasoning("openai.gpt-oss-120b", true, false)]
#[case::empty("", true, false)]
#[tokio::test]
async fn model_uses_max_completion_tokens_reads_the_vendor_rules(
    #[case] model: &str,
    #[case] reasoning: bool,
    #[case] expected: bool,
) {
    let outcome = uses_max_completion_tokens(model, &mut FakePredicates { reasoning }).await;
    assert_eq!(outcome, expected);
}
