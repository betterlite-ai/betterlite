use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_message_scope::scoped_indices::{ScanScope, ScopeFlags};
use crate::lakera_v2::Data;
use crate::lakera_v2::error::LakeraV2Error;

use super::{
    ATTACK_ONLY, CollectingSink, EMAIL_SPAN, Hook, MIXED, PII_ONLY, build, chat_data, content_of,
    context, flagged, invocation, message, run_hook, serve, update_params,
};

#[tokio::test]
async fn pre_call_masks_a_pii_only_verdict() {
    let server = serve(200, flagged(PII_ONLY, EMAIL_SPAN)).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    let mut data = chat_data();
    let outcome = run_hook(&guardrail, &mut data, None, Hook::Pre)
        .await
        .expect("allowed");
    assert_eq!(outcome.applied_guardrail.as_deref(), Some("lakera-v2"));
    assert_eq!(
        content_of(&data, 0).as_deref(),
        Some("email [MASKED EMAIL]")
    );
}

#[tokio::test]
async fn pre_call_blocks_a_mixed_verdict_in_block_mode() {
    let server = serve(200, flagged(MIXED, EMAIL_SPAN)).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    let mut data = chat_data();
    let error = run_hook(&guardrail, &mut data, None, Hook::Pre)
        .await
        .expect_err("blocked");
    let GuardrailFailure::HttpObject { status, detail } = error else {
        panic!("expected the blocked detail");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(
            "Violated guardrail policy".to_owned()
        ))
    );
    assert!(detail.get("lakera_guardrail_response").is_some());
}

#[tokio::test]
async fn pre_call_injects_the_advisory_after_masking_pii() {
    let server = serve(200, flagged(MIXED, EMAIL_SPAN)).await;
    let guardrail = build(
        &server.base_url,
        LakeraOnFlagged::InjectSystemMessage,
        Some("Flagged for {reason}."),
    );
    let mut data = chat_data();
    let outcome = run_hook(&guardrail, &mut data, None, Hook::Pre)
        .await
        .expect("advised");
    assert_eq!(outcome.applied_guardrail.as_deref(), Some("lakera-v2"));
    assert_eq!(
        content_of(&data, 0).as_deref(),
        Some("email [MASKED EMAIL]")
    );
    let Some(GuardrailValue::Array(messages)) = data.get("messages") else {
        panic!("messages are missing");
    };
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages.get(1),
        Some(&message(
            "system",
            "Flagged for personally identifiable information, a potential prompt injection attempt."
        ))
    );
}

#[tokio::test]
async fn pre_call_blocks_pii_on_multimodal_input_in_advisory_mode() {
    let server = serve(200, flagged(MIXED, EMAIL_SPAN)).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::InjectSystemMessage, None);
    let mut data = chat_data();
    data.insert(
        "input".to_owned(),
        GuardrailValue::Text("also this".to_owned()),
    );
    let error = run_hook(&guardrail, &mut data, None, Hook::Pre)
        .await
        .expect_err("blocked");
    assert!(matches!(
        error,
        GuardrailFailure::HttpObject { status: 400, .. }
    ));
    assert_eq!(content_of(&data, 0).as_deref(), Some("email a@example.com"));
}

#[tokio::test]
async fn pre_call_monitor_masks_the_instructions_pii() {
    let server = serve(200, flagged(PII_ONLY, EMAIL_SPAN)).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Monitor, None);
    let mut data = chat_data();
    data.insert(
        "instructions".to_owned(),
        GuardrailValue::Text("email a@example.com".to_owned()),
    );
    let outcome = run_hook(&guardrail, &mut data, None, Hook::Pre)
        .await
        .expect("allowed");
    assert_eq!(outcome.applied_guardrail.as_deref(), Some("lakera-v2"));
    assert_eq!(
        data.get("instructions"),
        Some(&GuardrailValue::Text("email [MASKED EMAIL]".to_owned()))
    );
    assert_eq!(content_of(&data, 0).as_deref(), Some("email a@example.com"));
}

#[tokio::test]
async fn pre_call_monitor_allows_a_non_pii_verdict() {
    let server = serve(200, flagged(ATTACK_ONLY, "[]")).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Monitor, None);
    let mut data = chat_data();
    let outcome = run_hook(&guardrail, &mut data, None, Hook::Pre)
        .await
        .expect("allowed");
    assert_eq!(outcome.applied_guardrail.as_deref(), Some("lakera-v2"));
    assert_eq!(content_of(&data, 0).as_deref(), Some("email a@example.com"));
}

#[tokio::test]
async fn pre_call_does_not_run_when_every_message_is_skipped() {
    let server = serve(200, flagged(ATTACK_ONLY, "[]")).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    let mut data = Data::new();
    data.insert(
        "messages".to_owned(),
        GuardrailValue::Array(vec![message("system", "you are a bot")]),
    );
    let sink = CollectingSink::default();
    let suppressed = BTreeSet::new();
    let mut call = invocation(&data);
    let scope = ScopeFlags {
        skip_system_message: true,
        skip_tool_message: false,
        scan: ScanScope::AllMessages,
    };
    let mut call_context = context(scope, &mut call, &sink, &suppressed);
    let outcome = guardrail
        .pre_call(&mut data, &mut call_context)
        .await
        .expect("skipped");
    assert_eq!(outcome.applied_guardrail, None);
    assert!(server.captured().is_empty());
}

#[tokio::test]
async fn during_call_never_masks_pii() {
    let server = serve(200, flagged(PII_ONLY, EMAIL_SPAN)).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Monitor, None);
    let mut data = chat_data();
    let outcome = run_hook(&guardrail, &mut data, None, Hook::During)
        .await
        .expect("allowed");
    assert_eq!(outcome.applied_guardrail.as_deref(), Some("lakera-v2"));
    assert_eq!(content_of(&data, 0).as_deref(), Some("email a@example.com"));
}

#[tokio::test]
async fn during_call_blocks_in_block_mode() {
    let server = serve(200, flagged(PII_ONLY, EMAIL_SPAN)).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    let mut data = chat_data();
    let error = run_hook(&guardrail, &mut data, None, Hook::During)
        .await
        .expect_err("blocked");
    assert!(matches!(
        error,
        GuardrailFailure::HttpObject { status: 400, .. }
    ));
}

#[tokio::test]
async fn during_call_blocks_pii_in_advisory_mode() {
    let server = serve(200, flagged(PII_ONLY, EMAIL_SPAN)).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::InjectSystemMessage, None);
    let mut data = chat_data();
    let error = run_hook(&guardrail, &mut data, None, Hook::During)
        .await
        .expect_err("pii blocks advisory mode");
    assert!(matches!(
        error,
        GuardrailFailure::HttpObject { status: 400, .. }
    ));
}

fn assistant_response() -> Data {
    let mut response = Data::new();
    response.insert(
        "choices".to_owned(),
        GuardrailValue::Array(vec![
            GuardrailValue::Object(BTreeMap::from([(
                "message".to_owned(),
                GuardrailValue::Object(BTreeMap::from([
                    (
                        "role".to_owned(),
                        GuardrailValue::Text("assistant".to_owned()),
                    ),
                    ("content".to_owned(), GuardrailValue::Null),
                ])),
            )])),
            GuardrailValue::Object(BTreeMap::from([(
                "message".to_owned(),
                GuardrailValue::Object(BTreeMap::from([
                    (
                        "role".to_owned(),
                        GuardrailValue::Text("assistant".to_owned()),
                    ),
                    (
                        "content".to_owned(),
                        GuardrailValue::Text("email a@example.com".to_owned()),
                    ),
                ])),
            )])),
        ]),
    );
    response
}

fn choice_content(response: &Data, index: usize) -> Option<GuardrailValue> {
    let Some(GuardrailValue::Array(choices)) = response.get("choices") else {
        return None;
    };
    let Some(GuardrailValue::Object(choice)) = choices.get(index) else {
        return None;
    };
    let Some(GuardrailValue::Object(message)) = choice.get("message") else {
        return None;
    };
    message.get("content").cloned()
}

#[tokio::test]
async fn post_call_masks_the_assistant_content_by_choice() {
    let server = serve(
        200,
        flagged(
            PII_ONLY,
            r#"[{"message_id":1,"start":6,"end":19,"detector_type":"pii/email"}]"#,
        ),
    )
    .await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    let mut data = chat_data();
    let mut response = assistant_response();
    let outcome = run_hook(&guardrail, &mut data, Some(&mut response), Hook::Post)
        .await
        .expect("allowed");
    assert_eq!(outcome.applied_guardrail.as_deref(), Some("lakera-v2"));
    assert_eq!(choice_content(&response, 0), Some(GuardrailValue::Null));
    assert_eq!(
        choice_content(&response, 1),
        Some(GuardrailValue::Text("email [MASKED EMAIL]".to_owned()))
    );
    assert_eq!(content_of(&data, 0).as_deref(), Some("email a@example.com"));
}

#[tokio::test]
async fn post_call_blocks_a_non_pii_verdict_in_block_mode() {
    let server = serve(200, flagged(ATTACK_ONLY, "[]")).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    let mut data = chat_data();
    let mut response = assistant_response();
    let error = run_hook(&guardrail, &mut data, Some(&mut response), Hook::Post)
        .await
        .expect_err("blocked");
    assert!(matches!(
        error,
        GuardrailFailure::HttpObject { status: 400, .. }
    ));
}

#[tokio::test]
async fn post_call_allows_a_non_pii_verdict_in_monitor_mode() {
    let server = serve(200, flagged(ATTACK_ONLY, "[]")).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Monitor, None);
    let mut data = chat_data();
    let mut response = assistant_response();
    let outcome = run_hook(&guardrail, &mut data, Some(&mut response), Hook::Post)
        .await
        .expect("allowed");
    assert_eq!(outcome.applied_guardrail.as_deref(), Some("lakera-v2"));
    assert_eq!(
        choice_content(&response, 1),
        Some(GuardrailValue::Text("email a@example.com".to_owned()))
    );
}

fn update_params(extra: &str) -> LitellmParams {
    let body = format!(r#"{{"guardrail": "lakera_v2", "mode": "post_call"{extra}}}"#);
    betterlite_config::sections::guardrail_params::decode::decode_litellm_params(body.as_bytes())
        .expect("valid params")
}

#[test]
fn update_refuses_the_escaped_template_and_keeps_the_instance() {
    let mut guardrail = build(
        "http://unused",
        LakeraOnFlagged::InjectSystemMessage,
        Some("ok {reason}"),
    );
    let params = update_params(r#", "advisory_system_message": "Watch out for {{reason}}""#);
    let error = guardrail
        .update_in_memory_litellm_params(&params)
        .expect_err("the template is invalid");
    assert_eq!(error.to_string(), ESCAPED_TEMPLATE_TEXT);
    assert_eq!(
        guardrail.advisory_system_message.as_deref(),
        Some("ok {reason}")
    );
    assert_eq!(guardrail.on_flagged, LakeraOnFlagged::InjectSystemMessage);
    assert!(guardrail.base.settings.event_hook.is_none());
}

#[test]
fn update_applies_the_advisory_fields_and_the_new_mode() {
    let mut guardrail = build("http://unused", LakeraOnFlagged::Block, None);
    let params = update_params(
        r#", "on_flagged": "inject_system_message", "advisory_system_message": "Why: {reason}", "payload": true, "breakdown": true"#,
    );
    guardrail
        .update_in_memory_litellm_params(&params)
        .expect("a valid update");
    assert_eq!(guardrail.on_flagged, LakeraOnFlagged::InjectSystemMessage);
    assert_eq!(
        guardrail.advisory_system_message.as_deref(),
        Some("Why: {reason}")
    );
    assert!(guardrail.flags.payload);
    assert!(guardrail.flags.breakdown);
    assert_eq!(
        guardrail.base.settings.event_hook,
        Some(LitellmParamsMode::One("post_call".to_owned()))
    );
}

#[test]
fn update_rejects_a_wrong_typed_payload() {
    let mut guardrail = build("http://unused", LakeraOnFlagged::Block, None);
    let params = update_params(r#", "payload": "yes""#);
    let error = guardrail
        .update_in_memory_litellm_params(&params)
        .expect_err("payload must be a bool");
    assert!(
        matches!(&error, LakeraV2Error::InvalidParam { field, .. } if field == "payload"),
        "{error}"
    );
    assert!(guardrail.flags.payload);
}

#[test]
fn timeout_is_resolved_from_the_configured_seconds() {
    let mut args = args();
    args.timeout = Some(2.5);
    let env = BTreeMap::new();
    let guardrail =
        LakeraV2Guardrail::new(args, base(true), reqwest::Client::default(), &env).expect("built");
    assert_eq!(guardrail.timeout, Some(Duration::from_millis(2500)));
}
