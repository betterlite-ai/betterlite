use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::support::{
    ATTACK, CLEAN, NoSecrets, build, guardrail, mock, params, seen_requests, text,
};
use crate::azure_prompt_shield::guardrail::SUPPORTED_EVENT_HOOKS;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;

fn inputs(texts: &[&str]) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

#[test]
fn the_supported_event_hooks_are_pre_call_and_during_call() {
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::DuringCall
        ]
    );
    let guardrail = guardrail("http://127.0.0.1:1");
    assert_eq!(
        guardrail.get_supported_event_hooks(),
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::DuringCall
        ])
    );
    assert_eq!(
        guardrail.base.settings.supported_event_hooks,
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::DuringCall
        ])
    );
}

#[test]
fn a_configured_hook_list_is_kept() {
    let mut params = params("http://127.0.0.1:1");
    params.settings.supported_event_hooks = Some(vec![GuardrailEventHooks::PostCall]);
    let guardrail = build(params, &NoSecrets);
    assert_eq!(
        guardrail.base.settings.supported_event_hooks,
        Some(vec![GuardrailEventHooks::PostCall])
    );
    assert_eq!(
        guardrail.get_supported_event_hooks(),
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::DuringCall
        ])
    );
}

#[test]
fn the_native_lifecycle_and_apply_guardrail_switches_are_on() {
    let guardrail = guardrail("http://127.0.0.1:1");
    assert!(guardrail.base.settings.switches.use_native_lifecycle_hooks);
    assert!(
        guardrail
            .base
            .settings
            .switches
            .uses_apply_guardrail_interface
    );
}

#[tokio::test]
async fn apply_guardrail_scans_every_text_and_returns_the_inputs() {
    let server = mock(vec![CLEAN]).await;
    let guardrail = guardrail(&server.base);
    let given = inputs(&["first prompt", "second prompt"]);
    let mut data = BTreeMap::new();
    let returned = guardrail
        .apply_guardrail(given.clone(), &mut data, GuardrailInputType::Request, None)
        .await
        .unwrap();
    assert_eq!(returned, given);
    assert_eq!(seen_requests(&server).len(), 2);
}

#[tokio::test]
async fn apply_guardrail_skips_empty_texts() {
    let server = mock(vec![CLEAN]).await;
    let guardrail = guardrail(&server.base);
    let given = inputs(&["", "hello"]);
    let returned = guardrail
        .apply_guardrail(
            given,
            &mut BTreeMap::new(),
            GuardrailInputType::Response,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        returned.texts,
        Some(vec![String::new(), "hello".to_owned()])
    );
    let seen = seen_requests(&server);
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen.first().unwrap().body,
        r#"{"userPrompt":"hello","documents":[]}"#
    );
}

#[tokio::test]
async fn apply_guardrail_without_texts_sends_nothing() {
    let server = mock(vec![CLEAN]).await;
    let guardrail = guardrail(&server.base);
    let returned = guardrail
        .apply_guardrail(
            inputs(&[]),
            &mut BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(returned.texts, Some(Vec::new()));
    assert!(seen_requests(&server).is_empty());
}

#[tokio::test]
async fn apply_guardrail_blocks_an_attack_with_a_400() {
    let server = mock(vec![ATTACK]).await;
    let guardrail = guardrail(&server.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["attack"]),
            &mut BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap_err();
    match error {
        GuardrailFailure::HttpObject { status, detail } => {
            assert_eq!(status, 400);
            assert_eq!(
                detail.get("error").cloned(),
                Some(text("Violated Azure Prompt Shield guardrail policy"))
            );
            assert_eq!(
                detail.get("detection_message").cloned(),
                Some(text(r#"Attack detected: {"attackDetected":true}"#))
            );
        }
        other => panic!("expected an object detail, got {other:?}"),
    }
}
