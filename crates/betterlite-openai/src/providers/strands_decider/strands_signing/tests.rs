#![cfg(test)]

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_models::decisions::ir;
use betterlite_models::decisions::{DecisionsJson, Extra, Json};
use rstest::rstest;
use secrecy::SecretString;

use super::StrandsDeciderDecisionsConfig;
use super::headers::SESSION_HEADER;
use super::signing::AwsCredentials;
use crate::decisions::transformation::provider_config::{
    DecisionsProviderConfig as _, SigningContext,
};
use crate::decisions::transformation::wire::RequestWire;
use crate::openai_base_config::Headers;

const ARN: &str = "arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/my-runtime";
const ACCESS_KEY: &str = "AKIDEXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
const SESSION_TOKEN: &str = "session-token";
const NOW: u64 = 1_767_225_600;
const DEFAULT_SESSION: &str =
    "litellm-decider-92976cdcb2dfb49b5759fc567c6cc5241c0c5d52482fd1e0d7db5116ae4c5c20";
const EXPECTED_BODY: &[u8] =
    br#"{"model": "m", "state": "the state", "questions": {"q": {"type": "noul", "instructions": "decide"}}}"#;
const EXPECTED_AUTHORIZATION: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-east-1/bedrock-agentcore/aws4_request, SignedHeaders=host;x-amz-date;x-amz-security-token;x-amzn-bedrock-agentcore-runtime-session-id;x-custom, Signature=119941b9f1ffca7c0b0d0b0dacd8d2b1da4c096df793d3d6893324439452f497";

fn with_clock(credentials: Option<AwsCredentials>) -> StrandsDeciderDecisionsConfig {
    StrandsDeciderDecisionsConfig {
        credentials,
        clock: Arc::new(|| {
            UNIX_EPOCH
                .checked_add(Duration::from_secs(NOW))
                .expect("a representable time")
        }),
    }
}

fn sample_wire(config: &StrandsDeciderDecisionsConfig) -> RequestWire {
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

fn static_credentials() -> Extra {
    let mut params = Extra::new();
    params.insert(
        "aws_access_key_id".to_owned(),
        Json::Text(ACCESS_KEY.to_owned()),
    );
    params.insert(
        "aws_secret_access_key".to_owned(),
        Json::Text(SECRET_KEY.to_owned()),
    );
    params.insert(
        "aws_session_token".to_owned(),
        Json::Text(SESSION_TOKEN.to_owned()),
    );
    params
}

fn custom_headers() -> Headers {
    let mut headers = Headers::new();
    headers.insert("X-Custom".to_owned(), "v".to_owned());
    headers
}

#[rstest]
#[case::from_litellm_params(false)]
#[case::injected_into_the_config(true)]
fn test_aws_signing_signs_the_json_body_for_the_bedrock_agentcore_service(#[case] injected: bool) {
    let credentials = injected.then(|| AwsCredentials {
        access_key_id: SecretString::from(ACCESS_KEY),
        secret_access_key: SecretString::from(SECRET_KEY),
        session_token: Some(SecretString::from(SESSION_TOKEN)),
    });
    let config = with_clock(credentials);
    let body = sample_wire(&config);
    let params = if injected {
        Extra::new()
    } else {
        static_credentials()
    };
    let url = config.get_complete_url(ARN, "m");
    let context = SigningContext {
        url: &url,
        api_base: ARN,
        body: &body,
        api_key: None,
        litellm_params: &params,
    };
    let (signed, payload) = config.sign_request(custom_headers(), context);
    assert_eq!(payload.as_deref(), Some(EXPECTED_BODY));
    assert_eq!(
        signed.get("Authorization").map(String::as_str),
        Some(EXPECTED_AUTHORIZATION)
    );
    assert_eq!(
        signed.get("X-Amz-Date").map(String::as_str),
        Some("20260101T000000Z")
    );
    assert_eq!(
        signed.get("X-Amz-Security-Token").map(String::as_str),
        Some(SESSION_TOKEN)
    );
    assert_eq!(
        signed.get("Content-Length").map(String::as_str),
        Some("100")
    );
    assert_eq!(
        signed.get(SESSION_HEADER).map(String::as_str),
        Some(DEFAULT_SESSION)
    );
    assert_eq!(signed.get("X-Custom").map(String::as_str), Some("v"));
}

#[rstest]
#[case::runtime_jwt("runtime-jwt")]
#[case::empty_token("")]
fn test_bearer_bypass_returns_the_session_headers_without_signing(#[case] token: &str) {
    let config = with_clock(None);
    let body = sample_wire(&config);
    let api_key = SecretString::from(token);
    let params = Extra::new();
    let url = config.get_complete_url(ARN, "m");
    let context = SigningContext {
        url: &url,
        api_base: ARN,
        body: &body,
        api_key: Some(&api_key),
        litellm_params: &params,
    };
    let (signed, payload) = config.sign_request(custom_headers(), context);
    assert!(payload.is_none());
    assert_eq!(signed.get("Authorization"), None);
    assert_eq!(signed.get("X-Amz-Date"), None);
    assert_eq!(
        signed.get(SESSION_HEADER).map(String::as_str),
        Some(DEFAULT_SESSION)
    );
    assert_eq!(signed.get("X-Custom").map(String::as_str), Some("v"));
}

#[test]
fn aws_signing_without_credentials_leaves_the_request_unsigned() {
    let config = with_clock(None);
    let body = sample_wire(&config);
    let params = Extra::new();
    let url = config.get_complete_url(ARN, "m");
    let context = SigningContext {
        url: &url,
        api_base: ARN,
        body: &body,
        api_key: None,
        litellm_params: &params,
    };
    let (signed, payload) = config.sign_request(custom_headers(), context);
    assert!(payload.is_none());
    assert_eq!(signed.get("Authorization"), None);
    assert_eq!(
        signed.get(SESSION_HEADER).map(String::as_str),
        Some(DEFAULT_SESSION)
    );
}
