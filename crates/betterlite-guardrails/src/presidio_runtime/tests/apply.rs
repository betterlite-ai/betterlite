use betterlite_config::sections::presidio_runtime::Settings as RuntimeSettings;

use super::support::{
    Replies, SOCIAL_SECURITY_FOUND, analyzer_requests, block, build, field, guard_args, guard_at,
    inputs, masking, mock, request, seen, text, tokens,
};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_initializers::presidio::PresidioRole;
use crate::presidio_runtime::PresidioDefaults;

const CALL_TEXT: &str = "call me at 555-0100";

pub(super) async fn masks_texts() {
    let server = masking();
    let guard = guard_at(&server);
    let mut data = request(r#"{"model":"gpt-4o"}"#);
    let masked = guard
        .apply_guardrail(
            inputs(Some(vec![CALL_TEXT])),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("the text is masked");
    assert_eq!(masked.texts, Some(vec!["<PHONE_NUMBER>".to_owned()]));
}

pub(super) async fn masks_texts_in_order() {
    let server = masking();
    let guard = guard_at(&server);
    let mut data = request(r#"{"model":"gpt-4o"}"#);
    let masked = guard
        .apply_guardrail(
            inputs(Some(vec!["first 555-0100", "second 555-0199"])),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("the texts are masked");
    assert_eq!(
        masked.texts,
        Some(vec![
            "<PHONE_NUMBER>".to_owned(),
            "<PHONE_NUMBER>".to_owned()
        ])
    );
    let analyzer_texts: Vec<String> = analyzer_requests(&server)
        .iter()
        .map(|request| text(field(request, "text")).to_owned())
        .collect();
    assert_eq!(analyzer_texts, vec!["first 555-0100", "second 555-0199"]);
}

pub(super) async fn adds_empty_texts() {
    let server = masking();
    let guard = guard_at(&server);
    let mut data = request(r#"{"model":"gpt-4o"}"#);
    let added = guard
        .apply_guardrail(inputs(None), &mut data, GuardrailInputType::Request, None)
        .await
        .expect("the answer gains an empty texts list");
    assert_eq!(added.texts, Some(Vec::new()));
    assert!(seen(&server).is_empty());
}

pub(super) async fn restores_on_a_restore_callback() {
    let server = masking();
    let mut args = guard_args(&server);
    args.role = PresidioRole::Restore;
    let guard = super::support::guard(&args);
    let mut data = tokens(&[("<PHONE_NUMBER_1>", "555-0100")]);
    let restored = guard
        .apply_guardrail(
            inputs(Some(vec!["call <PHONE_NUMBER_1>"])),
            &mut data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect("the text is restored");
    assert_eq!(restored.texts, Some(vec!["call 555-0100".to_owned()]));
    assert!(seen(&server).is_empty());
}

pub(super) async fn restores_on_a_roleless_callback() {
    let server = masking();
    let args = guard_args(&server);
    let guard = build(
        &args,
        PresidioDefaults::default(),
        &RuntimeSettings::default(),
        false,
        None,
    )
    .expect("the roleless guard");
    let mut data = tokens(&[("<PHONE_NUMBER_1>", "555-0100")]);
    let restored = guard
        .apply_guardrail(
            inputs(Some(vec!["call <PHONE_NUMBER_1>"])),
            &mut data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect("the text is restored");
    assert_eq!(restored.texts, Some(vec!["call 555-0100".to_owned()]));
    assert!(seen(&server).is_empty());
}

pub(super) async fn masks_a_roleless_callback_without_tokens() {
    let server = masking();
    let args = guard_args(&server);
    let guard = build(
        &args,
        PresidioDefaults::default(),
        &RuntimeSettings::default(),
        false,
        None,
    )
    .expect("the roleless guard");
    let mut data = request(r#"{"model":"gpt-4o"}"#);
    let masked = guard
        .apply_guardrail(
            inputs(Some(vec![CALL_TEXT])),
            &mut data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect("the text is masked");
    assert_eq!(masked.texts, Some(vec!["<PHONE_NUMBER>".to_owned()]));
}

pub(super) async fn masks_a_roleless_callback_that_masks_output() {
    let server = masking();
    let mut args = guard_args(&server);
    args.apply_to_output = true;
    let guard = build(
        &args,
        PresidioDefaults::default(),
        &RuntimeSettings::default(),
        false,
        None,
    )
    .expect("the roleless guard");
    let mut data = tokens(&[("<PHONE_NUMBER_1>", "555-0100")]);
    let masked = guard
        .apply_guardrail(
            inputs(Some(vec![CALL_TEXT])),
            &mut data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .expect("the text is masked");
    assert_eq!(masked.texts, Some(vec!["<PHONE_NUMBER>".to_owned()]));
}

pub(super) async fn masks_on_a_request_input() {
    let server = masking();
    let mut args = guard_args(&server);
    args.role = PresidioRole::Restore;
    let guard = super::support::guard(&args);
    let mut data = tokens(&[("<PHONE_NUMBER_1>", "555-0100")]);
    let masked = guard
        .apply_guardrail(
            inputs(Some(vec![CALL_TEXT])),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("a request input is masked");
    assert_eq!(masked.texts, Some(vec!["<PHONE_NUMBER>".to_owned()]));
}

pub(super) async fn blocked_entity_fails_apply() {
    let server = mock(Replies {
        analyze: (200, SOCIAL_SECURITY_FOUND),
        anonymize: (200, super::support::MASKED),
    });
    let mut args = guard_args(&server);
    args.pii_entities_config = Some(block("US_SSN"));
    let guard = super::support::guard(&args);
    let mut data = request(r#"{"model":"gpt-4o"}"#);
    let error = guard
        .apply_guardrail(
            inputs(Some(vec!["1234 is secret"])),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("a blocked entity fails the call");
    assert!(matches!(error, GuardrailFailure::BlockedPii { .. }));
}
