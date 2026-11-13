#![cfg(test)]

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_models::decisions::ir;
use betterlite_models::decisions::{DecisionsJson, Extra, Json};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use super::StrandsDeciderDecisionsConfig;
use super::arn::agentcore_runtime;
use super::headers::SESSION_HEADER;
use crate::decisions::transformation::provider_config::{
    DecisionsProviderConfig as _, SigningContext,
};
use crate::decisions::transformation::wire::ProviderRequestWire;
use crate::openai_base_config::Headers;

const ARN: &str = "arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/my-runtime";
const OTHER_ARN: &str = "arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/other";
const DEFAULT_SESSION: &str =
    "litellm-decider-92976cdcb2dfb49b5759fc567c6cc5241c0c5d52482fd1e0d7db5116ae4c5c20";
const NOW: u64 = 1_767_225_600;

fn fixed_clock() -> Arc<dyn Fn() -> SystemTime + Send + Sync> {
    Arc::new(|| {
        UNIX_EPOCH
            .checked_add(Duration::from_secs(NOW))
            .expect("a representable time")
    })
}

fn sample_wire(config: &StrandsDeciderDecisionsConfig) -> ProviderRequestWire {
    let request = ir::DecisionRequest {
        input: ir::DecisionInput::State(DecisionsJson::Text("the state".to_owned())),
        questions: vec![ir::DecisionQuestion::Predicate(ir::PredicateQuestion {
            name: Some("q".to_owned()),
            instructions: Some(DecisionsJson::Text("decide".to_owned())),
            criteria: None,
            extra: Extra::default(),
        })],
        safety_identifier: None,
    };
    config
        .transform_decisions_request("m", &request, "strands_decider")
        .expect("a systemone body")
}

fn session_header(api_key: &SecretString, params: &Extra, body: &ProviderRequestWire) -> Headers {
    let config = StrandsDeciderDecisionsConfig {
        credentials: None,
        clock: fixed_clock(),
    };
    let context = SigningContext {
        url: "https://bedrock-agentcore.us-east-1.amazonaws.com/runtimes/x/invocations",
        api_base: ARN,
        body,
        api_key: Some(api_key),
        litellm_params: params,
    };
    let (signed, payload) = config.sign_request(Headers::new(), context);
    assert!(payload.is_none());
    signed
}

#[rstest]
#[case::canonical_runtime(ARN, DEFAULT_SESSION)]
#[case::other_runtime(
    OTHER_ARN,
    "litellm-decider-3d4c9b8003ceb5bb1e41470a294c944f20d619763cf29df3beb2f1098bc74ec8"
)]
fn test_default_session_id_is_the_litellm_sha256_of_the_arn(
    #[case] arn: &str,
    #[case] expected: &str,
) {
    let runtime = agentcore_runtime(arn).expect("a runtime ARN");
    assert_eq!(runtime.default_session_id(), expected);
}

#[test]
fn default_session_id_is_stable_per_arn_and_has_the_agentcore_length() {
    let first = agentcore_runtime(ARN).expect("a runtime ARN");
    let again = agentcore_runtime(ARN).expect("a runtime ARN");
    assert_eq!(first.default_session_id(), again.default_session_id());
    assert_eq!(
        first
            .default_session_id()
            .strip_prefix("litellm-decider-")
            .map(str::len),
        Some(64)
    );
}

#[rstest]
#[case::configured("replica-session-a")]
#[case::empty("")]
fn test_override_session_id_from_litellm_params_becomes_the_session_header(
    #[case] configured: &str,
) {
    let api_key = SecretString::from("runtime-jwt");
    let mut params = Extra::new();
    params.insert(
        "agentcore_runtime_session_id".to_owned(),
        Json::Text(configured.to_owned()),
    );
    let body = sample_wire(&StrandsDeciderDecisionsConfig::default());
    let signed = session_header(&api_key, &params, &body);
    assert_eq!(
        signed.get(SESSION_HEADER).map(String::as_str),
        Some(configured)
    );
}

#[test]
fn override_session_id_null_keeps_the_default_session_header() {
    let api_key = SecretString::from("runtime-jwt");
    let mut params = Extra::new();
    params.insert("agentcore_runtime_session_id".to_owned(), Json::Null);
    let body = sample_wire(&StrandsDeciderDecisionsConfig::default());
    let signed = session_header(&api_key, &params, &body);
    assert_eq!(
        signed.get(SESSION_HEADER).map(String::as_str),
        Some(DEFAULT_SESSION)
    );
}

#[test]
fn override_session_id_of_another_type_keeps_the_default_session_header() {
    let api_key = SecretString::from("runtime-jwt");
    let mut params = Extra::new();
    params.insert(
        "agentcore_runtime_session_id".to_owned(),
        Json::Number(Number::from(7_u8)),
    );
    let body = sample_wire(&StrandsDeciderDecisionsConfig::default());
    let signed = session_header(&api_key, &params, &body);
    assert_eq!(
        signed.get(SESSION_HEADER).map(String::as_str),
        Some(DEFAULT_SESSION)
    );
}

#[test]
fn session_header_name_matches_litellm_casing() {
    assert_eq!(
        SESSION_HEADER,
        "X-Amzn-Bedrock-AgentCore-Runtime-Session-Id"
    );
}
