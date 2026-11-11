#![cfg(test)]

use betterlite_models::decisions::ir;
use betterlite_models::decisions::{DecisionsJson, Extra};
use rstest::rstest;

use super::StrandsDeciderDecisionsConfig;
use super::headers::SESSION_HEADER;
use crate::decisions::transformation::provider_config::{
    DecisionsProviderConfig as _, SigningContext,
};
use crate::decisions::transformation::wire::ProviderRequestWire;
use crate::openai_base_config::Headers;

const ARN: &str = "arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/my-runtime";
const RUNTIME_URL: &str = "https://bedrock-agentcore.us-east-1.amazonaws.com/runtimes/\
                           arn%3Aaws%3Abedrock-agentcore%3Aus-east-1%3A123456789012%3Aruntime%2Fmy-runtime/invocations";

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

#[rstest]
#[case::plain("https://strands.example", "https://strands.example/v1/systemone")]
#[case::version_suffix("https://strands.example/v1", "https://strands.example/v1/systemone")]
fn test_direct_http_fallback_keeps_the_parent_url_and_skips_aws_signing(
    #[case] api_base: &str,
    #[case] expected: &str,
) {
    let config = StrandsDeciderDecisionsConfig::default();
    assert!(!config.signs_with_aws(api_base));
    assert_eq!(config.get_complete_url(api_base, "m"), expected);
    assert!(config.signs_with_aws(ARN));
    assert_eq!(config.get_complete_url(ARN, "m"), RUNTIME_URL);
}

#[test]
fn direct_http_fallback_returns_the_caller_headers_without_a_signed_body() {
    let config = StrandsDeciderDecisionsConfig::default();
    let body = sample_wire(&config);
    let params = Extra::new();
    let mut headers = Headers::new();
    headers.insert("Authorization".to_owned(), "Bearer runtime-jwt".to_owned());
    headers.insert("X-Custom".to_owned(), "v".to_owned());
    let context = SigningContext {
        url: "https://strands.example/v1/systemone",
        api_base: "https://strands.example",
        body: &body,
        api_key: None,
        litellm_params: &params,
    };
    let (signed, payload) = config.sign_request(headers.clone(), context);
    assert!(payload.is_none());
    assert_eq!(signed, headers);
    assert!(!signed.contains_key(SESSION_HEADER));
}
