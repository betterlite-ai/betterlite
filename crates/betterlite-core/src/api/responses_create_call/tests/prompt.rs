use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::prompting::CARRY_UNMATCHED_MESSAGE_POINTS;
use crate::api::responses_provider_selection::provider_config::ASYNC_PROMPT_MERGED_PARAMS;

use super::super::error::AsyncCreateCause;
use super::super::hook::run_hook;
use super::support::{self, Outcome, PortAnswer, ScriptedHandoff, ScriptedPort, ScriptedResolver};

const PROMPT: &str = r#"{"model":"openai/gpt-4o","input":"hello","kwargs":{"prompt_id":"p","prompt_label":"production","prompt_version":2,"prompt_variables":{"name":"Ada"}}}"#;
const SAME_MODEL: &str = r#"{"model":"openai/gpt-4o","input":"hello","kwargs":{"prompt_id":"p","model_info":{"id":"deployment_a"}}}"#;
const PINNED: &str = r#"{"model":"openai/gpt-4o","input":"hello","api_key":"sk-test-secret","kwargs":{"prompt_id":"p"}}"#;
const BRIDGED: &str =
    r#"{"model":"groq/llama-3.3-70b","input":"hello","kwargs":{"prompt_id":"p"}}"#;
const SYSTEM: &str = r#"{"role":"system","content":"Be brief."}"#;
const USER: &str = r#"{"role":"user","content":"hello"}"#;

pub(super) async fn async_prompt_hooks() {
    runs_once().await;
    no_logging_object().await;
    ineligible_logging_object().await;
}

pub(super) async fn prompt_outcomes() {
    same_model().await;
    changed_provider().await;
    credential_boundary().await;
    bridged_signal().await;
    carry_removed_on_error().await;
    non_text_prompt_id().await;
}

// One async hook merges the prompt, the sync entry consumes the outcome once and never re-runs the hook
async fn runs_once() {
    let call = support::call_from(PROMPT);
    let answer = support::with_params(
        support::answer("openai/gpt-4o-mini", &[SYSTEM, USER]),
        r#"{"temperature":0.2}"#,
    );
    let port = ScriptedPort::answering(true, PortAnswer::Answer(answer));
    let handoff = ScriptedHandoff::new(Outcome::Completed(Some(support::stamped("resp_a"))));
    let resolver = ScriptedResolver::default();
    support::drive(&resolver, Some(&port), &handoff, &call)
        .await
        .expect("the prompt merges");
    one_hook(&port);
    merged_outcome(&handoff);
    let requests = resolver.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].model, "openai/gpt-4o-mini");
    assert_eq!(requests[1].custom_llm_provider, None);
}

fn one_hook(port: &ScriptedPort) {
    let records = port.records();
    assert_eq!(port.checks(), 1);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].model, "openai/gpt-4o");
    assert_eq!(records[0].prompt_id.as_deref(), Some("p"));
    assert_eq!(records[0].prompt_label.as_deref(), Some("production"));
    assert_eq!(records[0].prompt_version, Some(support::json("2")));
    assert_eq!(
        records[0].prompt_variables,
        Some(support::json(r#"{"name":"Ada"}"#))
    );
    assert_eq!(records[0].client_messages, vec![support::json(USER)]);
    assert!(!records[0].carry);
    assert!(records[0].aresponses);
}

fn merged_outcome(handoff: &ScriptedHandoff) {
    let calls = handoff.calls();
    let merged = &calls[0];
    assert_eq!(merged.model, "openai/gpt-4o-mini");
    assert_eq!(merged.custom_llm_provider.as_deref(), Some("openai"));
    assert!(!merged.kwargs.contains_key("prompt_id"));
    assert_eq!(
        support::input_json(merged),
        support::json(&format!("[{SYSTEM},{USER}]"))
    );
    assert_eq!(
        merged.kwargs.get(ASYNC_PROMPT_MERGED_PARAMS),
        Some(&support::value(
            r#"{"merged_optional_params":{"temperature":0.2},"deployment_model_info":null}"#
        ))
    );
}

// Without an eligible logging object no hook runs, so prompt_id stays for the sync entry and no outcome is consumed
async fn no_logging_object() {
    let call = support::call_from(PROMPT);
    let handoff = ScriptedHandoff::new(Outcome::Completed(Some(support::stamped("resp_a"))));
    let resolver = ScriptedResolver::default();
    support::drive(&resolver, None, &handoff, &call)
        .await
        .expect("the call answers");
    let calls = handoff.calls();
    assert_eq!(calls[0].model, "openai/gpt-4o");
    assert_eq!(
        calls[0].kwargs.get("prompt_id"),
        Some(&support::value("\"p\""))
    );
    assert!(!calls[0].kwargs.contains_key(ASYNC_PROMPT_MERGED_PARAMS));
    assert_eq!(resolver.requests().len(), 1);
}

// An ineligible logging object's false verdict leaves prompt_id and the input for the sync entry
async fn ineligible_logging_object() {
    let call = support::call_from(PROMPT);
    let answer = support::answer("openai/gpt-4o-mini", &[SYSTEM, USER]);
    let port = ScriptedPort::answering(false, PortAnswer::Answer(answer));
    let handoff = ScriptedHandoff::new(Outcome::Completed(Some(support::stamped("resp_a"))));
    let resolver = ScriptedResolver::default();
    support::drive(&resolver, Some(&port), &handoff, &call)
        .await
        .expect("the call answers");
    assert_eq!(port.checks(), 1);
    assert_eq!(port.checked_prompt_ids(), vec![Some(String::from("p"))]);
    assert!(port.records().is_empty());
    let calls = handoff.calls();
    assert_eq!(calls[0].model, "openai/gpt-4o");
    assert_eq!(
        calls[0].kwargs.get("prompt_id"),
        Some(&support::value("\"p\""))
    );
    assert!(!calls[0].kwargs.contains_key(ASYNC_PROMPT_MERGED_PARAMS));
    assert_eq!(support::input_json(&calls[0]), support::json("\"hello\""));
    assert_eq!(resolver.requests().len(), 1);
}

async fn same_model() {
    let call = support::call_from(SAME_MODEL);
    let port = ScriptedPort::answering(
        true,
        PortAnswer::Answer(support::answer("openai/gpt-4o", &[USER])),
    );
    let handoff = ScriptedHandoff::new(Outcome::Completed(Some(support::stamped("resp_a"))));
    let resolver = ScriptedResolver::default();
    support::drive(&resolver, Some(&port), &handoff, &call)
        .await
        .expect("the prompt merges");
    assert_eq!(resolver.requests().len(), 1);
    let calls = handoff.calls();
    assert_eq!(calls[0].model, "openai/gpt-4o");
    assert_eq!(
        calls[0].kwargs.get(ASYNC_PROMPT_MERGED_PARAMS),
        Some(&support::value(
            r#"{"merged_optional_params":{},"deployment_model_info":{"id":"deployment_a"}}"#
        ))
    );
}

async fn changed_provider() {
    let call = support::call_from(SAME_MODEL);
    let port = ScriptedPort::answering(
        true,
        PortAnswer::Answer(support::answer("anthropic/claude-sonnet-4", &[USER])),
    );
    let handoff = ScriptedHandoff::new(Outcome::Completed(Some(support::stamped("resp_a"))));
    let resolver = ScriptedResolver::default();
    support::drive(&resolver, Some(&port), &handoff, &call)
        .await
        .expect("the prompt merges");
    let calls = handoff.calls();
    assert_eq!(calls[0].model, "anthropic/claude-sonnet-4");
    assert_eq!(calls[0].custom_llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(
        calls[0].kwargs.get(ASYNC_PROMPT_MERGED_PARAMS),
        Some(&support::value(
            r#"{"merged_optional_params":{},"deployment_model_info":null}"#
        ))
    );
}

// A provider change with credentials already resolved is refused before the sync call runs
async fn credential_boundary() {
    let call = support::call_from(PINNED);
    let port = ScriptedPort::answering(
        true,
        PortAnswer::Answer(support::answer("anthropic/claude-sonnet-4", &[USER])),
    );
    let handoff = ScriptedHandoff::new(Outcome::Completed(Some(support::stamped("resp_a"))));
    let resolver = ScriptedResolver::default();
    let error = match support::drive(&resolver, Some(&port), &handoff, &call).await {
        Ok(_created) => panic!("the swap is refused"),
        Err(error) => error,
    };
    assert!(matches!(
        *error.cause,
        AsyncCreateCause::CredentialBoundary(_)
    ));
    assert_eq!(error.provider, "anthropic");
    assert_eq!(error.model, "anthropic/claude-sonnet-4");
    let failure = support::failure(&error);
    assert_eq!(failure.class, "BadRequestError");
    assert_eq!(failure.status, 400);
    assert!(
        failure.message.starts_with(
            "litellm.BadRequestError: prompt_id 'p' swaps model 'openai/gpt-4o' -> 'anthropic/claude-sonnet-4'"
        ),
        "{}",
        failure.message
    );
    assert!(
        failure
            .message
            .contains("changes the provider from 'openai' to 'anthropic'")
    );
    assert_eq!(handoff.calls().len(), 0);
}

async fn bridged_signal() {
    let call = support::call_from(BRIDGED);
    let port = ScriptedPort::answering(
        true,
        PortAnswer::Answer(support::answer("groq/llama-3.3-70b", &[USER])),
    );
    let handoff = ScriptedHandoff::new(Outcome::Completed(Some(support::stamped("resp_a"))));
    let resolver = ScriptedResolver::default();
    support::drive(&resolver, Some(&port), &handoff, &call)
        .await
        .expect("the prompt merges");
    assert!(port.records()[0].carry);
    let calls = handoff.calls();
    assert_eq!(calls[0].custom_llm_provider.as_deref(), Some("groq"));
    assert!(!calls[0].kwargs.contains_key(CARRY_UNMATCHED_MESSAGE_POINTS));
}

async fn carry_removed_on_error() {
    let call = support::call_from(BRIDGED);
    let port = ScriptedPort::answering(true, PortAnswer::Fail(String::from("the hook failed")));
    let mut kwargs = ParameterMap::new();
    let error = run_hook(&port, &call, &mut kwargs, &[], true)
        .await
        .expect_err("the hook fails");
    assert_eq!(error.message, "the hook failed");
    assert!(port.records()[0].carry);
    assert!(!kwargs.contains_key(CARRY_UNMATCHED_MESSAGE_POINTS));
}

// LiteLLM's cast leaves a non-string prompt_id as it is; the port declares a string, so it counts as absent
async fn non_text_prompt_id() {
    let call =
        support::call_from(r#"{"model":"openai/gpt-4o","input":"hello","kwargs":{"prompt_id":7}}"#);
    let port = ScriptedPort::answering(
        true,
        PortAnswer::Answer(support::answer("openai/gpt-4o", &[USER])),
    );
    let handoff = ScriptedHandoff::new(Outcome::Completed(Some(support::stamped("resp_a"))));
    let resolver = ScriptedResolver::default();
    support::drive(&resolver, Some(&port), &handoff, &call)
        .await
        .expect("the prompt merges");
    assert_eq!(port.checked_prompt_ids(), vec![None]);
}
