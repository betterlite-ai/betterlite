use betterlite_core::routes::CallType;

use super::support::routes;
use crate::guardrail_hooks::{RESPONSES_API_CALL_TYPES, is_responses_api_route};

#[test]
fn the_constant_holds_both_responses_call_types() {
    assert_eq!(
        RESPONSES_API_CALL_TYPES,
        [CallType::Responses, CallType::Aresponses]
    );
}

#[test]
fn a_responses_route_is_detected() {
    let table = routes("/v1/responses", vec![CallType::Responses]);
    assert!(is_responses_api_route(Some("/v1/responses"), &table));
}

#[test]
fn an_async_responses_route_is_detected() {
    let table = routes("/v1/responses", vec![CallType::Aresponses]);
    assert!(is_responses_api_route(Some("/v1/responses"), &table));
}

#[test]
fn a_chat_route_is_not_a_responses_route() {
    let table = routes("/v1/chat/completions", vec![CallType::Completion]);
    assert!(!is_responses_api_route(
        Some("/v1/chat/completions"),
        &table
    ));
}

#[test]
fn a_route_whose_call_types_mix_in_responses_is_detected() {
    let table = routes(
        "/v1/responses",
        vec![CallType::Responses, CallType::Completion],
    );
    assert!(is_responses_api_route(Some("/v1/responses"), &table));
}

#[test]
fn no_request_route_is_never_a_responses_route() {
    let table = routes("/v1/responses", vec![CallType::Responses]);
    assert!(!is_responses_api_route(None, &table));
}

#[test]
fn an_unknown_route_is_not_a_responses_route() {
    let table = routes("/v1/responses", vec![CallType::Responses]);
    assert!(!is_responses_api_route(Some("/v1/unknown"), &table));
}
