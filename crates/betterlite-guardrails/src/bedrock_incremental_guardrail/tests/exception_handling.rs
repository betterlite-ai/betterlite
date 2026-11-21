#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;

use super::batch::wrap_unexpected;
use crate::guardrail_base::failure::{
    GuardrailFailure, ModifyResponseFailure, SensitiveDataRouteFailure,
};

#[test]
fn an_http_failure_passes_through() {
    let failure = GuardrailFailure::Http {
        status: 400,
        detail: "invalid content".to_owned(),
    };

    assert_eq!(wrap_unexpected(failure.clone()), failure);
}

#[test]
fn an_http_object_failure_passes_through() {
    let failure = GuardrailFailure::HttpObject {
        status: 400,
        detail: IndexMap::from([(
            "error".to_owned(),
            GuardrailValue::Text("Violated guardrail policy".to_owned()),
        )]),
    };

    assert_eq!(wrap_unexpected(failure.clone()), failure);
}

#[test]
fn a_modify_response_failure_passes_through() {
    let failure = GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
        message: "blocked text".to_owned(),
        model: "bedrock-guardrail".to_owned(),
        request_data: BTreeMap::new(),
        guardrail_name: Some("bedrock-incremental".to_owned()),
        detection_info: BTreeMap::new(),
        original_response: None,
    }));

    assert_eq!(wrap_unexpected(failure.clone()), failure);
}

#[test]
fn a_guardrail_raised_failure_passes_through() {
    let failure = GuardrailFailure::GuardrailRaised {
        guardrail_name: Some("bedrock-incremental".to_owned()),
        message: "blocked text".to_owned(),
        wrap_default_message: false,
        status_code: 400,
        blocked_content: true,
    };

    assert_eq!(wrap_unexpected(failure.clone()), failure);
}

#[test]
fn a_blocked_pii_failure_passes_through() {
    let failure = GuardrailFailure::BlockedPii {
        entity_type: "EMAIL".to_owned(),
        guardrail_name: Some("bedrock-incremental".to_owned()),
        status_code: 400,
    };

    assert_eq!(wrap_unexpected(failure.clone()), failure);
}

#[test]
fn a_sensitive_data_route_failure_passes_through() {
    let failure = GuardrailFailure::SensitiveDataRoute(Box::new(SensitiveDataRouteFailure {
        route_to_model: "safe-model".to_owned(),
        session_id: "session-1".to_owned(),
        guardrail_name: Some("bedrock-incremental".to_owned()),
        detection_info: BTreeMap::new(),
        message: None,
        sticky_session_routing: false,
    }));

    assert_eq!(wrap_unexpected(failure.clone()), failure);
}

#[test]
fn an_unexpected_failure_is_wrapped() {
    let failure = wrap_unexpected(GuardrailFailure::Other("boom".to_owned()));

    assert_eq!(
        failure,
        GuardrailFailure::Other("Bedrock guardrail failed: boom".to_owned())
    );
}
