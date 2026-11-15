use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_initializers::lakera_v2::LakeraV2Args;
use crate::guardrail_message_scope::scoped_indices::{ScanScope, ScopeFlags};
use crate::lakera_v2::Data;
use crate::lakera_v2::error::LakeraV2Error;
use crate::lakera_v2::guardrail::LakeraV2Guardrail;

use super::{
    ATTACK_ONLY, CollectingSink, EMAIL_SPAN, ESCAPED_TEMPLATE_TEXT, Hook, MIXED, PII_ONLY, args,
    base, body_of, build, chat_data, content_of, context, flagged, invocation, message, run_hook,
    serve,
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
    let Some(GuardrailValue::Object(carried)) = detail.get("lakera_guardrail_response") else {
        panic!("the Lakera answer is not carried: {detail:?}");
    };
    assert_eq!(carried.get("flagged"), Some(&GuardrailValue::Boolean(true)));
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
async fn pre_call_monitor_forwards_multimodal_pii_it_cannot_mask() {
    let server = serve(200, flagged(PII_ONLY, EMAIL_SPAN)).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Monitor, None);
    let mut data = chat_data();
    data.insert(
        "messages".to_owned(),
        GuardrailValue::Array(vec![GuardrailValue::Object(BTreeMap::from([
            ("role".to_owned(), GuardrailValue::Text("user".to_owned())),
            ("content".to_owned(), text_part("email a@example.com")),
        ]))]),
    );
    let original = data.clone();
    let outcome = run_hook(&guardrail, &mut data, None, Hook::Pre)
        .await
        .expect("the unredacted request is forwarded");
    assert_eq!(outcome.applied_guardrail.as_deref(), Some("lakera-v2"));
    assert_eq!(data, original);
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

#[tokio::test]
async fn during_call_allows_a_non_pii_verdict_in_advisory_mode() {
    let server = serve(200, flagged(ATTACK_ONLY, "[]")).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::InjectSystemMessage, None);
    let mut data = chat_data();
    let outcome = run_hook(&guardrail, &mut data, None, Hook::During)
        .await
        .expect("advisory mode has nothing to inject during during_call");
    assert_eq!(outcome.applied_guardrail.as_deref(), Some("lakera-v2"));
    assert_eq!(content_of(&data, 0).as_deref(), Some("email a@example.com"));
}

fn assistant_response() -> Data {
    assistant_response_with(GuardrailValue::Null)
}

fn assistant_response_with(first_content: GuardrailValue) -> Data {
    response_with_choices(vec![
        first_content,
        GuardrailValue::Text("email a@example.com".to_owned()),
    ])
}

fn response_with_choices(contents: Vec<GuardrailValue>) -> Data {
    let choices = contents
        .into_iter()
        .map(|content| {
            GuardrailValue::Object(BTreeMap::from([(
                "message".to_owned(),
                GuardrailValue::Object(BTreeMap::from([
                    (
                        "role".to_owned(),
                        GuardrailValue::Text("assistant".to_owned()),
                    ),
                    ("content".to_owned(), content),
                ])),
            )]))
        })
        .collect();
    let mut response = Data::new();
    response.insert("choices".to_owned(), GuardrailValue::Array(choices));
    response
}

fn text_part(text: &str) -> GuardrailValue {
    GuardrailValue::Array(vec![GuardrailValue::Object(BTreeMap::from([
        ("type".to_owned(), GuardrailValue::Text("text".to_owned())),
        ("text".to_owned(), GuardrailValue::Text(text.to_owned())),
    ]))])
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
    let params = update_params(
        r#", "on_flagged": "inject_system_message", "advisory_system_message": "Watch out for {{reason}}""#,
    );
    let error = guardrail
        .update_params(&params)
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
    guardrail.update_params(&params).expect("a valid update");
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
fn update_keeps_the_connection_fields() {
    let mut args = args();
    args.api_base = Some("http://live".to_owned());
    args.api_key = Some(SecretString::from("live-key"));
    args.project_id = Some("live-project".to_owned());
    args.metadata = Some(BTreeMap::from([(
        "team".to_owned(),
        GuardrailValue::Text("a".to_owned()),
    )]));
    let mut guardrail = built(args);
    let params = update_params(
        r#", "api_base": "http://other", "api_key": "other", "project_id": "other", "metadata": {"team": "b"}"#,
    );
    guardrail.update_params(&params).expect("a valid update");
    assert_eq!(guardrail.api_base, "http://live");
    assert_eq!(guardrail.api_key.expose_secret(), "live-key");
    assert_eq!(guardrail.project_id.as_deref(), Some("live-project"));
    assert_eq!(
        guardrail
            .metadata
            .as_ref()
            .and_then(|fields| fields.get("team")),
        Some(&GuardrailValue::Text("a".to_owned()))
    );
}

#[test]
fn update_rejects_a_wrong_typed_payload() {
    let mut guardrail = build("http://unused", LakeraOnFlagged::Block, None);
    let params = update_params(r#", "payload": "yes""#);
    let error = guardrail
        .update_params(&params)
        .expect_err("payload must be a bool");
    assert!(
        matches!(&error, LakeraV2Error::InvalidParam { field, .. } if field == "payload"),
        "{error}"
    );
    assert!(guardrail.flags.payload);
}

fn built(args: LakeraV2Args) -> LakeraV2Guardrail {
    let env = BTreeMap::new();
    LakeraV2Guardrail::new(args, base(true), reqwest::Client::default(), &env).expect("built")
}

#[test]
fn update_resets_omitted_params_to_the_model_defaults() {
    let mut args = args();
    args.on_flagged = Some(LakeraOnFlagged::Monitor);
    args.payload = Some(false);
    args.breakdown = Some(false);
    args.dev_info = Some(false);
    let mut guardrail = built(args);
    guardrail
        .update_params(&update_params(""))
        .expect("a valid update");
    assert_eq!(guardrail.on_flagged, LakeraOnFlagged::Block);
    assert!(guardrail.flags.payload);
    assert!(guardrail.flags.breakdown);
    assert!(guardrail.flags.dev_info);
}

#[test]
fn update_keeps_the_live_values_for_explicit_nulls() {
    let mut args = args();
    args.on_flagged = Some(LakeraOnFlagged::Monitor);
    args.payload = Some(false);
    args.breakdown = Some(false);
    args.dev_info = Some(false);
    let mut guardrail = built(args);
    let params = update_params(
        r#", "on_flagged": null, "payload": null, "breakdown": null, "dev_info": null"#,
    );
    guardrail.update_params(&params).expect("a valid update");
    assert_eq!(guardrail.on_flagged, LakeraOnFlagged::Monitor);
    assert!(!guardrail.flags.payload);
    assert!(!guardrail.flags.breakdown);
    assert!(!guardrail.flags.dev_info);
}

#[test]
fn update_to_advisory_mode_defaults_an_omitted_payload_to_true() {
    let mut args = args();
    args.payload = Some(false);
    let mut guardrail = built(args);
    let params = update_params(
        r#", "on_flagged": "inject_system_message", "advisory_system_message": "Why: {reason}""#,
    );
    guardrail
        .update_params(&params)
        .expect("the omitted payload defaults to true");
    assert_eq!(guardrail.on_flagged, LakeraOnFlagged::InjectSystemMessage);
    assert!(guardrail.flags.payload);
}

#[test]
fn timeout_is_resolved_from_the_configured_seconds() {
    let mut args = args();
    args.timeout = Some(2.5);
    assert_eq!(built(args).timeout, Duration::from_millis(2500));
}

#[test]
fn the_default_timeout_is_lite_llms_six_hundred_seconds() {
    assert_eq!(built(args()).timeout, Duration::from_secs(600));
}

#[test]
fn update_resets_an_omitted_timeout_to_the_default() {
    let mut args = args();
    args.timeout = Some(2.5);
    let mut guardrail = built(args);
    guardrail
        .update_params(&update_params(""))
        .expect("a valid update");
    assert_eq!(guardrail.timeout, Duration::from_secs(600));
}

#[tokio::test]
async fn post_call_leaves_out_choices_with_an_empty_content() {
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
    let mut response = assistant_response_with(GuardrailValue::Text(String::new()));
    run_hook(&guardrail, &mut data, Some(&mut response), Hook::Post)
        .await
        .expect("allowed");
    assert_eq!(
        choice_content(&response, 0),
        Some(GuardrailValue::Text(String::new()))
    );
    assert_eq!(
        choice_content(&response, 1),
        Some(GuardrailValue::Text("email [MASKED EMAIL]".to_owned()))
    );
}

#[tokio::test]
async fn post_call_sends_only_choices_with_content_to_the_guard() {
    let server = serve(200, flagged(ATTACK_ONLY, "[]")).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    let mut data = chat_data();
    let mut response = response_with_choices(vec![
        GuardrailValue::Null,
        GuardrailValue::Text(String::new()),
        GuardrailValue::Array(Vec::new()),
        GuardrailValue::Text("email a@example.com".to_owned()),
    ]);
    run_hook(&guardrail, &mut data, Some(&mut response), Hook::Post)
        .await
        .expect_err("blocked");
    let body = body_of(&server);
    let Some(GuardrailValue::Array(messages)) = body.get("messages") else {
        panic!("the guard request has no messages: {body:?}");
    };
    assert_eq!(messages.len(), 2);
    let Some(GuardrailValue::Object(assistant)) = messages.get(1) else {
        panic!("the assistant message is missing: {messages:?}");
    };
    assert_eq!(
        assistant.get("content"),
        Some(&GuardrailValue::Text("email a@example.com".to_owned()))
    );
}

#[tokio::test]
async fn post_call_blocks_a_choice_with_a_text_part_list() {
    let server = serve(200, flagged(ATTACK_ONLY, "[]")).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    let mut data = chat_data();
    let list = text_part("email a@example.com");
    let mut response = response_with_choices(vec![list.clone()]);
    let error = run_hook(&guardrail, &mut data, Some(&mut response), Hook::Post)
        .await
        .expect_err("blocked");
    assert!(matches!(
        error,
        GuardrailFailure::HttpObject { status: 400, .. }
    ));
    let body = body_of(&server);
    let Some(GuardrailValue::Array(messages)) = body.get("messages") else {
        panic!("the guard request has no messages: {body:?}");
    };
    assert_eq!(messages.len(), 2);
    let Some(GuardrailValue::Object(assistant)) = messages.get(1) else {
        panic!("the assistant message is missing: {messages:?}");
    };
    assert_eq!(assistant.get("content"), Some(&list));
}

#[tokio::test]
async fn post_call_keeps_a_text_part_list_unmasked() {
    let server = serve(
        200,
        flagged(
            PII_ONLY,
            r#"[{"message_id":2,"start":6,"end":19,"detector_type":"pii/email"}]"#,
        ),
    )
    .await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    let mut data = chat_data();
    let list = text_part("email a@example.com");
    let mut response = response_with_choices(vec![
        list.clone(),
        GuardrailValue::Text("email a@example.com".to_owned()),
    ]);
    let outcome = run_hook(&guardrail, &mut data, Some(&mut response), Hook::Post)
        .await
        .expect("the list is left as it is");
    assert_eq!(outcome.applied_guardrail.as_deref(), Some("lakera-v2"));
    assert_eq!(choice_content(&response, 0), Some(list));
    assert_eq!(
        choice_content(&response, 1),
        Some(GuardrailValue::Text("email [MASKED EMAIL]".to_owned()))
    );
}

#[tokio::test]
async fn post_call_masks_the_choice_after_a_skipped_system_message() {
    let server = serve(
        200,
        flagged(
            PII_ONLY,
            r#"[{"message_id":1,"start":6,"end":19,"detector_type":"pii/email"}]"#,
        ),
    )
    .await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    let mut data = Data::new();
    data.insert(
        "messages".to_owned(),
        GuardrailValue::Array(vec![
            message("system", "secret a@example.com"),
            message("user", "email a@example.com"),
        ]),
    );
    let mut response =
        response_with_choices(vec![GuardrailValue::Text("email a@example.com".to_owned())]);
    let sink = CollectingSink::default();
    let suppressed = BTreeSet::new();
    let mut call = invocation(&data);
    let scope = ScopeFlags {
        skip_system_message: true,
        skip_tool_message: false,
        scan: ScanScope::AllMessages,
    };
    let mut call_context = context(scope, &mut call, &sink, &suppressed);
    guardrail
        .post_call(&mut data, &mut response, &mut call_context)
        .await
        .expect("masked");
    assert_eq!(
        choice_content(&response, 0),
        Some(GuardrailValue::Text("email [MASKED EMAIL]".to_owned()))
    );
    let Some(GuardrailValue::Array(messages)) = data.get("messages") else {
        panic!("the request messages are missing");
    };
    assert_eq!(
        messages.get(0),
        Some(&message("system", "secret a@example.com"))
    );
    assert_eq!(
        messages.get(1),
        Some(&message("user", "email a@example.com"))
    );
}

#[rstest]
#[case::pre_call(Hook::Pre)]
#[case::during_call(Hook::During)]
#[case::post_call(Hook::Post)]
#[tokio::test]
async fn hooks_report_the_applied_guardrail_for_an_unflagged_answer(#[case] hook: Hook) {
    let server = serve(
        200,
        r#"{"flagged":false,"payload":[],"breakdown":[]}"#.to_owned(),
    )
    .await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    let mut data = chat_data();
    let mut response = assistant_response();
    let outcome = run_hook(&guardrail, &mut data, Some(&mut response), hook)
        .await
        .expect("unflagged");
    assert_eq!(outcome.applied_guardrail.as_deref(), Some("lakera-v2"));
    assert_eq!(content_of(&data, 0).as_deref(), Some("email a@example.com"));
    assert_eq!(response, assistant_response());
}

#[rstest]
#[case::pre_call(Hook::Pre)]
#[case::during_call(Hook::During)]
#[case::post_call(Hook::Post)]
#[tokio::test]
async fn hooks_skip_a_guardrail_the_request_did_not_select(#[case] hook: Hook) {
    let server = serve(200, flagged(PII_ONLY, EMAIL_SPAN)).await;
    let mut guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    guardrail.base.settings.default_on = false;
    guardrail.base.settings.event_hook = Some(LitellmParamsMode::Many(vec![
        "pre_call".to_owned(),
        "during_call".to_owned(),
        "post_call".to_owned(),
    ]));
    let mut data = chat_data();
    let mut response = assistant_response();
    let outcome = run_hook(&guardrail, &mut data, Some(&mut response), hook)
        .await
        .expect("not selected");
    assert_eq!(outcome.applied_guardrail, None);
    assert_eq!(data, chat_data());
    assert!(server.captured().is_empty());
}
