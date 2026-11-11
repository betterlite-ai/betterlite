#![cfg(test)]

use betterlite_models::decisions::ir;
use betterlite_models::decisions::{DecisionsJson, Extra};

use super::StrandsDeciderDecisionsConfig;
use crate::decisions::transformation::provider_config::DecisionsProviderConfig as _;

fn lookups(variable: &str) -> String {
    match variable {
        "STRANDS_DECIDER_API_KEY" => "runtime-jwt".to_owned(),
        "STRANDS_DECIDER_API_BASE" => "https://strands.example".to_owned(),
        _ => String::new(),
    }
}

#[test]
fn strands_config_uses_the_strands_decider_environment_names() {
    assert_eq!(
        StrandsDeciderDecisionsConfig::API_KEY_ENV,
        ["STRANDS_DECIDER_API_KEY"]
    );
    assert_eq!(
        StrandsDeciderDecisionsConfig::API_BASE_ENV,
        ["STRANDS_DECIDER_API_BASE"]
    );
}

#[test]
fn strands_config_requires_no_api_key_and_has_no_default_api_base() {
    let config = StrandsDeciderDecisionsConfig::default();
    let required: bool = StrandsDeciderDecisionsConfig::API_KEY_REQUIRED;
    assert!(!required);
    assert_eq!(config.get_default_api_base(), None);
    assert_eq!(config.resolve_api_key(None, |_| String::new()), None);
    assert_eq!(config.resolve_api_base(None, |_| String::new()), None);
}

#[test]
fn strands_config_resolves_the_key_and_base_from_their_environment_names() {
    let config = StrandsDeciderDecisionsConfig::default();
    assert_eq!(
        config.resolve_api_key(None, lookups).as_deref(),
        Some("runtime-jwt")
    );
    assert_eq!(
        config.resolve_api_base(None, lookups).as_deref(),
        Some("https://strands.example")
    );
}

#[test]
fn strands_config_inherits_the_systemone_request_body_of_the_parent() {
    let config = StrandsDeciderDecisionsConfig::default();
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
    let wire = config
        .transform_decisions_request("m", &request, "strands_decider")
        .expect("a systemone body");
    let rendered = super::body::request_body(&wire).expect("python style json");
    assert_eq!(
        rendered.as_slice(),
        br#"{"model": "m", "state": "the state", "questions": {"q": {"type": "noul", "instructions": "decide"}}}"#
    );
}
