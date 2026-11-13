use std::sync::Arc;

use super::support::chat_request;
use super::support::guardrail_hook;
use super::support::hook;
use super::support::request;
use crate::call_setup::guardrails::get_applied_guardrails;
use crate::call_setup::guardrails::get_request_guardrails;
use crate::call_setup::hooks::DeploymentHook;

#[test]
fn guardrail_list_takes_a_default_on_guardrail() {
    let hooks: Vec<Arc<dyn DeploymentHook>> = vec![guardrail_hook("g", Some("pii"), true)];

    let applied = get_applied_guardrails(&chat_request("{}"), &hooks).expect("the request reads");

    assert_eq!(applied, vec![String::from("pii")]);
}

#[test]
fn guardrail_list_takes_a_requested_guardrail() {
    let hooks: Vec<Arc<dyn DeploymentHook>> = vec![guardrail_hook("g", Some("pii"), false)];
    let request = chat_request(r#"{"metadata":{"requester_metadata":{"guardrails":["pii"]}}}"#);

    let applied = get_applied_guardrails(&request, &hooks).expect("the request reads");

    assert_eq!(applied, vec![String::from("pii")]);
}

#[test]
fn guardrail_list_skips_a_guardrail_nobody_asked_for() {
    let hooks: Vec<Arc<dyn DeploymentHook>> = vec![
        guardrail_hook("g1", Some("pii"), false),
        guardrail_hook("g2", Some("other"), false),
    ];
    let request = chat_request(r#"{"metadata":{"requester_metadata":{"guardrails":["other"]}}}"#);

    let applied = get_applied_guardrails(&request, &hooks).expect("the request reads");

    assert_eq!(applied, vec![String::from("other")]);
}

#[test]
fn guardrail_list_skips_an_unnamed_guardrail() {
    let hooks: Vec<Arc<dyn DeploymentHook>> = vec![guardrail_hook("g", None, true)];

    let applied = get_applied_guardrails(&chat_request("{}"), &hooks).expect("the request reads");

    assert!(applied.is_empty());
}

#[test]
fn guardrail_list_skips_a_plain_logger() {
    let hooks: Vec<Arc<dyn DeploymentHook>> = vec![hook("logger")];

    let applied = get_applied_guardrails(&chat_request("{}"), &hooks).expect("the request reads");

    assert!(applied.is_empty());
}

#[test]
fn guardrail_list_keeps_duplicate_names() {
    let hooks: Vec<Arc<dyn DeploymentHook>> = vec![
        guardrail_hook("g1", Some("pii"), true),
        guardrail_hook("g2", Some("pii"), true),
    ];

    let applied = get_applied_guardrails(&chat_request("{}"), &hooks).expect("the request reads");

    assert_eq!(applied, vec![String::from("pii"), String::from("pii")]);
}

#[test]
fn guardrail_list_reads_no_guardrails_from_a_plain_metadata() {
    let request = chat_request(r#"{"metadata":{"team":"a"}}"#);

    let guardrails = get_request_guardrails(&request).expect("the request reads");

    assert!(guardrails.is_empty());
}

#[test]
fn guardrail_list_request_names_come_from_requester_metadata() {
    let request = request(
        r#"{"metadata":{"requester_metadata":{"guardrails":["a","b"]}},"litellm_metadata":{"requester_metadata":{"guardrails":["c"]}}}"#,
    );

    let guardrails = get_request_guardrails(&request).expect("the request reads");

    assert_eq!(guardrails, vec![String::from("a"), String::from("b")]);
}

#[test]
fn guardrail_list_rejects_a_guardrail_list_of_non_strings() {
    let request = chat_request(r#"{"metadata":{"requester_metadata":{"guardrails":[7]}}}"#);

    let error = get_request_guardrails(&request).expect_err("a number is not a guardrail name");

    assert_eq!(
        error.to_string(),
        "the metadata.requester_metadata.guardrails value has the wrong type"
    );
}

#[test]
fn guardrail_list_rejects_a_text_metadata() {
    let request = chat_request(r#"{"metadata":"team"}"#);

    let error = get_request_guardrails(&request).expect_err("text is not a metadata object");

    assert_eq!(error.to_string(), "the metadata value has the wrong type");
}

#[test]
fn guardrail_list_reads_a_null_metadata_as_absent() {
    let request = chat_request(r#"{"metadata":null,"litellm_metadata":null}"#);

    assert_eq!(
        get_request_guardrails(&request).expect("the request reads"),
        Vec::<String>::new()
    );
}
