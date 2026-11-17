use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::support::{
    MASKED, Replies, SOCIAL_SECURITY_FOUND, analyzer_requests, block, field, guard, guard_args,
    guard_at, inputs, masking, mock, output_masking_roleless_guard, request, restore_guard,
    roleless, seen, text, tokens,
};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract as _;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::presidio_runtime::PresidioGuard;

type Data = BTreeMap<String, GuardrailValue>;
type Kind = GuardrailInputType;

const MODEL_REQUEST: &str = r#"{"model":"gpt-4o"}"#;
const CALL_TEXT: &str = "call me at 555-0100";
const RESTORE_TEXT: &str = "call <PHONE_NUMBER_1>";
const TOKEN: &str = "<PHONE_NUMBER_1>";
const ORIGINAL: &str = "555-0100";

async fn masked_texts(
    guard: &PresidioGuard,
    wanted: Option<Vec<&str>>,
    data: &mut Data,
    kind: Kind,
) -> Option<Vec<String>> {
    guard
        .apply_guardrail(inputs(wanted), data, kind, None)
        .await
        .expect("the guardrail runs")
        .texts
}

fn phone() -> Vec<String> {
    vec!["<PHONE_NUMBER>".to_owned()]
}

fn phones() -> Vec<String> {
    vec!["<PHONE_NUMBER>".to_owned(), "<PHONE_NUMBER>".to_owned()]
}

fn restored() -> Vec<String> {
    vec!["call 555-0100".to_owned()]
}

async fn restored_by(guard: &PresidioGuard) -> Option<Vec<String>> {
    let mut data = tokens(&[(TOKEN, ORIGINAL)]);
    masked_texts(guard, Some(vec![RESTORE_TEXT]), &mut data, Kind::Response).await
}

pub(super) async fn masks_texts() {
    let server = masking();
    let guard = guard_at(&server);
    let mut data = request(MODEL_REQUEST);
    let texts = masked_texts(&guard, Some(vec![CALL_TEXT]), &mut data, Kind::Request).await;
    assert_eq!(texts, Some(phone()));
}

pub(super) async fn masks_texts_in_order() {
    let server = masking();
    let mut data = request(MODEL_REQUEST);
    let wanted = Some(vec!["first 555-0100", "second 555-0199"]);
    let texts = masked_texts(&guard_at(&server), wanted, &mut data, Kind::Request).await;
    assert_eq!(texts, Some(phones()));
    let analyzer_texts: Vec<String> = analyzer_requests(&server)
        .iter()
        .map(|request| text(field(request, "text")).to_owned())
        .collect();
    assert_eq!(analyzer_texts, vec!["first 555-0100", "second 555-0199"]);
}

pub(super) async fn adds_empty_texts() {
    let server = masking();
    let mut data = request(MODEL_REQUEST);
    let texts = masked_texts(&guard_at(&server), None, &mut data, Kind::Request).await;
    assert_eq!(texts, Some(Vec::new()));
    assert!(seen(&server).is_empty());
}

pub(super) async fn restores_on_a_restore_callback() {
    let server = masking();
    let texts = restored_by(&restore_guard(&server)).await;
    assert_eq!(texts, Some(restored()));
    assert!(seen(&server).is_empty());
}

pub(super) async fn restores_on_a_roleless_callback() {
    let server = masking();
    let texts = restored_by(&roleless(&guard_args(&server))).await;
    assert_eq!(texts, Some(restored()));
    assert!(seen(&server).is_empty());
}

pub(super) async fn masks_a_roleless_callback_without_tokens() {
    let server = masking();
    let mut data = request(MODEL_REQUEST);
    let guard = roleless(&guard_args(&server));
    let texts = masked_texts(&guard, Some(vec![CALL_TEXT]), &mut data, Kind::Response).await;
    assert_eq!(texts, Some(phone()));
}

pub(super) async fn masks_a_roleless_callback_that_masks_output() {
    let server = masking();
    let guard = output_masking_roleless_guard(&server);
    let mut data = tokens(&[(TOKEN, ORIGINAL)]);
    let texts = masked_texts(&guard, Some(vec![CALL_TEXT]), &mut data, Kind::Response).await;
    assert_eq!(texts, Some(phone()));
}

pub(super) async fn masks_on_a_request_input() {
    let server = masking();
    let guard = restore_guard(&server);
    let mut data = tokens(&[(TOKEN, ORIGINAL)]);
    let texts = masked_texts(&guard, Some(vec![CALL_TEXT]), &mut data, Kind::Request).await;
    assert_eq!(texts, Some(phone()));
}

pub(super) async fn blocked_entity_fails_apply() {
    let server = mock(Replies {
        analyze: (200, SOCIAL_SECURITY_FOUND),
        anonymize: (200, MASKED),
    });
    let mut args = guard_args(&server);
    args.pii_entities_config = Some(block("US_SSN"));
    let error = guard(&args)
        .apply_guardrail(
            inputs(Some(vec!["1234 is secret"])),
            &mut request(MODEL_REQUEST),
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("a blocked entity fails the call");
    assert!(matches!(error, GuardrailFailure::BlockedPii { .. }));
}
