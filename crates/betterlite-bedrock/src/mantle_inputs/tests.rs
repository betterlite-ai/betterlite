#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret;

use crate::mantle_inputs::chat_endpoint::{
    ChatEndpointInputs, ChatEndpointParams, chat_endpoint_inputs,
};
use crate::mantle_inputs::credentials::resolve_bearer_token;
use crate::mantle_inputs::error::MantleInputError;
use crate::mantle_inputs::records::MantleGlobals;

mod bearer_cases;
mod capability_cases;
mod chat_cases;
mod region_cases;
mod support;

#[derive(Debug, Clone, Copy)]
enum Scenario {
    BearerPrecedence,
    RegionPrefix,
    RegionResolution,
    ModelCapabilities,
    ChatInputs,
}

#[rstest]
#[case::mantle_bearer_precedence(Scenario::BearerPrecedence)]
#[case::mantle_region_prefix(Scenario::RegionPrefix)]
#[case::mantle_region_resolution(Scenario::RegionResolution)]
#[case::mantle_model_capabilities(Scenario::ModelCapabilities)]
#[case::mantle_chat_endpoint_inputs(Scenario::ChatInputs)]
fn mantle_inputs_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::BearerPrecedence => bearer_cases::mantle_bearer_precedence(),
        Scenario::RegionPrefix => region_cases::mantle_region_prefix(),
        Scenario::RegionResolution => region_cases::mantle_region_resolution(),
        Scenario::ModelCapabilities => capability_cases::mantle_model_capabilities(),
        Scenario::ChatInputs => chat_cases::mantle_chat_endpoint_inputs(),
    }
}

fn resolve(globals: &MantleGlobals) -> ChatEndpointInputs {
    chat_endpoint_inputs(
        ChatEndpointParams {
            model: Some("openai.gpt-5"),
            api_base: None,
            api_key: None,
            aws_region_name: None,
        },
        globals,
        &support::empty_cost(),
    )
    .unwrap()
}

fn key(inputs: &ChatEndpointInputs) -> Option<&str> {
    inputs.api_key.as_ref().map(ExposeSecret::expose_secret)
}

#[test]
fn mantle_inputs_credential_identity_isolation() {
    let first = support::globals(&[
        ("BEDROCK_MANTLE_API_KEY", "first-fixture-key"),
        ("BEDROCK_MANTLE_REGION", "eu-west-1"),
    ]);
    let second = support::globals(&[
        ("BEDROCK_MANTLE_API_KEY", "second-fixture-key"),
        ("BEDROCK_MANTLE_REGION", "ap-south-1"),
    ]);
    let first_inputs = resolve(&first);
    let second_inputs = resolve(&second);
    let first_key = key(&first_inputs);
    let second_key = key(&second_inputs);
    assert_eq!(first_key, Some("first-fixture-key"));
    assert_eq!(second_key, Some("second-fixture-key"));
    assert_eq!(
        first_inputs.api_base,
        "https://bedrock-mantle.eu-west-1.api.aws/v1"
    );
    assert_eq!(
        second_inputs.api_base,
        "https://bedrock-mantle.ap-south-1.api.aws/v1"
    );
    assert!(!second_inputs.api_base.contains("eu-west-1"));
    assert!(!first_inputs.api_base.contains("ap-south-1"));
    let shared = support::secret("shared-fixture-key");
    let shared_token = resolve_bearer_token(
        Some(&shared),
        first.mantle_api_key.as_ref(),
        first.aws_bearer_token.as_ref(),
    )
    .unwrap();
    assert_eq!(shared_token.expose_secret(), "shared-fixture-key");
    assert_ne!(first_key, second_key);
    assert!(
        second_inputs
            .api_key
            .as_ref()
            .is_some_and(|key| key.expose_secret() != "shared-fixture-key")
    );
}

#[test]
fn mantle_inputs_secret_safe_failure() {
    let secrets = ["fixture-mantle-key", "fixture-standard-token"];
    let globals = support::globals(&[
        ("BEDROCK_MANTLE_API_KEY", secrets[0]),
        ("AWS_BEARER_TOKEN_BEDROCK", secrets[1]),
        (
            "BEDROCK_MANTLE_API_BASE",
            "https://bedrock-mantle.us-east-1.api.aws",
        ),
    ]);
    let resolved = resolve(&globals);
    let failure = chat_endpoint_inputs(
        ChatEndpointParams {
            model: Some("openai.gpt-5"),
            api_base: None,
            api_key: None,
            aws_region_name: Some("BAD_REGION"),
        },
        &globals,
        &support::empty_cost(),
    )
    .unwrap_err();
    assert!(matches!(failure, MantleInputError::Region(_)));
    let rendered = format!("{globals:?} {resolved:?} {failure} {failure:?}");
    assert!(!rendered.contains(secrets[0]));
    assert!(!rendered.contains(secrets[1]));
}
