#![cfg(test)]

use secrecy::{ExposeSecret, SecretString};

use super::ConverseCall;
use super::target::{append_endpoint, converse_model_id};
use crate::converse_request::{ConverseMessageSettings, ConverseSettings};
use crate::converse_response::wire::ModelResponse;

#[test]
fn model_id_keeps_the_wrapper_without_a_region() {
    let model = converse_model_id("bedrock/converse/anthropic.claude-v2", None);
    assert_eq!(model.model_id, "bedrock%2Fconverse%2Fanthropic.claude-v2");
    assert_eq!(model.region, None);
}

#[test]
fn model_id_extracts_region_and_removes_nova_prefix() {
    let model = converse_model_id("bedrock/us-east-1/nova/model", None);
    assert_eq!(model.region.as_deref(), Some("us-east-1"));
    assert_eq!(model.model_id, "model");
}

#[test]
fn explicit_model_id_is_encoded_without_region_extraction() {
    let model = converse_model_id("bedrock/us-east-1/ignored", Some("model/id"));
    assert_eq!(model.model_id, "model%2Fid");
    assert_eq!(model.region, None);
}

#[test]
fn endpoint_suffix_preserves_trailing_slashes() {
    assert_eq!(
        append_endpoint("https://example.test/", "id", false),
        "https://example.test//model/id/converse"
    );
    assert_eq!(
        append_endpoint("https://example.test/", "id", true),
        "https://example.test//model/id/converse-stream"
    );
}

#[test]
fn bedrock_converse_call_secret_safe_failure() {
    let api_key = SecretString::from("fixture-secret-key".to_owned());
    let bearer = SecretString::from("fixture-bearer".to_owned());
    let call = call(&api_key, &bearer);
    let debug = format!("{call:?}");
    assert!(!debug.contains(api_key.expose_secret()));
    assert!(!debug.contains(bearer.expose_secret()));
}

fn call<'call>(api_key: &'call SecretString, bearer: &'call SecretString) -> ConverseCall<'call> {
    ConverseCall {
        model: "bedrock/converse/anthropic.claude-v2",
        messages: &[],
        optional_params: Default::default(),
        litellm_params: Default::default(),
        extra_headers: None,
        api_base: None,
        api_key: Some(api_key),
        timeout: None,
        acompletion: false,
        control: Default::default(),
        skip_pre_call_logging: false,
        model_response: ModelResponse {
            id: "response-id".to_owned(),
            created: 0,
            model: "model".to_owned(),
            object: "chat.completion".to_owned(),
            system_fingerprint: None,
            choices: Vec::new(),
            usage: None,
            service_tier: None,
            trace: None,
        },
        settings: ConverseSettings {
            global_drop: false,
            messages: ConverseMessageSettings {
                modify_params: false,
                neutralize_orphans: true,
            },
            request_metadata: None,
        },
        environment: super::ConverseEnvironment {
            bearer_token: Some(bearer),
            ..Default::default()
        },
        now: std::time::SystemTime::UNIX_EPOCH,
    }
}
