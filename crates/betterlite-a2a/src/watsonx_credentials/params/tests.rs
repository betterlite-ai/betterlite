#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{WxoLitellmParams, extract_litellm_params};

fn parse(json: &str) -> WxoLitellmParams {
    serde_json::from_str(json).unwrap()
}

#[test]
fn extracts_every_key_and_keeps_the_given_auth_mode() {
    let params = extract_litellm_params(parse(
        r#"{"cp4d_host": "https://host", "instance_id": "i1", "wxo_agent_id": "a1", "api_key": "k",
            "username": "u", "auth_mode": "ibm_cloud", "thread_id": "t1", "model": "ignored"}"#,
    ))
    .unwrap();
    assert_eq!(params.cp4d_host, "https://host");
    assert_eq!(params.instance_id, "i1");
    assert_eq!(params.wxo_agent_id, "a1");
    assert_eq!(params.api_key.expose_secret(), "k");
    assert_eq!(params.username.as_deref(), Some("u"));
    assert_eq!(params.auth_mode, "ibm_cloud");
    assert_eq!(params.thread_id.as_deref(), Some("t1"));
}

#[rstest]
#[case::absent(r#"{"cp4d_host": "h", "instance_id": "i", "wxo_agent_id": "a", "api_key": "k"}"#)]
#[case::empty_strings(
    r#"{"cp4d_host": "h", "instance_id": "i", "wxo_agent_id": "a", "api_key": "k",
        "username": "", "auth_mode": "", "thread_id": ""}"#
)]
#[case::nulls(
    r#"{"cp4d_host": "h", "instance_id": "i", "wxo_agent_id": "a", "api_key": "k",
        "username": null, "auth_mode": null, "thread_id": null}"#
)]
fn optional_keys_default_to_cp4d_without_username_or_thread(#[case] json: &str) {
    let params = extract_litellm_params(parse(json)).unwrap();
    assert_eq!(params.auth_mode, "cp4d");
    assert_eq!(params.username, None);
    assert_eq!(params.thread_id, None);
}

#[rstest]
#[case::cp4d_host(
    r#"{"instance_id": "i", "wxo_agent_id": "a", "api_key": "k"}"#,
    "cp4d_host"
)]
#[case::instance_id(
    r#"{"cp4d_host": "h", "wxo_agent_id": "a", "api_key": "k"}"#,
    "instance_id"
)]
#[case::wxo_agent_id(
    r#"{"cp4d_host": "h", "instance_id": "i", "api_key": "k"}"#,
    "wxo_agent_id"
)]
#[case::api_key(
    r#"{"cp4d_host": "h", "instance_id": "i", "wxo_agent_id": "a"}"#,
    "api_key"
)]
#[case::empty_api_key(
    r#"{"cp4d_host": "h", "instance_id": "i", "wxo_agent_id": "a", "api_key": ""}"#,
    "api_key"
)]
#[case::null_host(
    r#"{"cp4d_host": null, "instance_id": "i", "wxo_agent_id": "a", "api_key": "k"}"#,
    "cp4d_host"
)]
#[case::first_missing_is_reported(r#"{"api_key": "k"}"#, "cp4d_host")]
fn missing_required_key_is_named(#[case] json: &str, #[case] key: &str) {
    assert_eq!(extract_litellm_params(parse(json)).unwrap_err().key, key);
}

#[test]
fn wrong_typed_key_is_rejected_by_the_decoder() {
    let error = serde_json::from_str::<WxoLitellmParams>(r#"{"cp4d_host": 5}"#).unwrap_err();
    assert!(error.to_string().starts_with("invalid type: integer `5`"));
}
