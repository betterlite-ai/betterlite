use betterlite_config::sections::content_filter_records::BlockCodeAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::block_code_execution::registration::ApplyGuardrailError;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;

use crate::block_code_execution::support::{
    apply, apply_capturing, guardrail, inputs, params, request_data,
};

#[test]
fn a_missing_texts_key_comes_back_unchanged() {
    let built = guardrail(params());
    let mut data = request_data();
    let mut given = inputs(&[]);
    given.texts = None;
    let returned = apply(&built, given, &mut data, GuardrailInputType::Request).expect("unchanged");
    assert!(returned.texts.is_none());
}

#[test]
fn an_empty_text_list_comes_back_as_an_empty_list() {
    let built = guardrail(params());
    let mut data = request_data();
    let returned =
        apply(&built, inputs(&[]), &mut data, GuardrailInputType::Request).expect("unchanged");
    assert_eq!(returned.texts, Some(Vec::new()));
}

#[test]
fn a_mask_action_replaces_a_blocked_block() {
    let mut configured = params();
    configured.blocked_languages = Some(vec!["python".to_owned()]);
    configured.action = BlockCodeAction::Mask;
    let built = guardrail(configured);
    let mut data = request_data();
    let returned = apply(
        &built,
        inputs(&["before ```python\nprint(1)\n``` after"]),
        &mut data,
        GuardrailInputType::Response,
    )
    .expect("masked");
    assert_eq!(
        returned.texts,
        Some(vec!["before [CODE_BLOCK_REDACTED] after".to_owned()])
    );
}

#[test]
fn an_unlisted_block_is_left_alone() {
    let mut configured = params();
    configured.blocked_languages = Some(vec!["python".to_owned()]);
    let built = guardrail(configured);
    let mut data = request_data();
    let text = "```rust\nfn main() {}\n```";
    let returned = apply(
        &built,
        inputs(&[text]),
        &mut data,
        GuardrailInputType::Response,
    )
    .expect("allowed");
    assert_eq!(returned.texts, Some(vec![text.to_owned()]));
}

#[test]
fn a_request_without_execution_intent_is_untouched() {
    let built = guardrail(params());
    let mut data = request_data();
    let text = "Don't run this:\n```python\nprint(1)\n```";
    let returned = apply(
        &built,
        inputs(&[text]),
        &mut data,
        GuardrailInputType::Request,
    )
    .expect("allowed");
    assert_eq!(returned.texts, Some(vec![text.to_owned()]));
}

#[test]
fn a_blocked_request_stops_at_the_first_raising_text() {
    let built = guardrail(params());
    let mut data = request_data();
    let first = "run this ```python\nprint(1)\n```";
    let second = "and run this ```bash\nls\n```";
    let (outcome, records) = apply_capturing(
        &built,
        inputs(&[first, second]),
        &mut data,
        GuardrailInputType::Request,
    );
    let Err(ApplyGuardrailError::Guardrail(GuardrailFailure::ModifyResponse(detail))) = outcome
    else {
        panic!("a blocked request must pass through");
    };
    assert_eq!(
        detail.detection_info.get("language"),
        Some(&GuardrailValue::Text("python".to_owned()))
    );
    let record = &records[0];
    let GuardrailValue::Array(detections) = &record.guardrail_response else {
        panic!(
            "expected detection dicts, got {:?}",
            record.guardrail_response
        );
    };
    assert_eq!(detections.len(), 1);
}

#[test]
fn a_blocked_response_returns_the_http_object() {
    let built = guardrail(params());
    let mut data = request_data();
    let (outcome, _records) = apply_capturing(
        &built,
        inputs(&["```python\nprint(1)\n```"]),
        &mut data,
        GuardrailInputType::Response,
    );
    let Err(ApplyGuardrailError::Guardrail(GuardrailFailure::HttpObject { status, detail })) =
        outcome
    else {
        panic!("a blocked response must be an object failure");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("language"),
        Some(&GuardrailValue::Text("python".to_owned()))
    );
}

#[test]
fn every_processed_text_reaches_the_returned_inputs() {
    let mut configured = params();
    configured.blocked_languages = Some(vec!["python".to_owned()]);
    let built = guardrail(configured);
    let mut data = request_data();
    let returned = apply(
        &built,
        inputs(&["```rust\na\n```", "plain text"]),
        &mut data,
        GuardrailInputType::Response,
    )
    .expect("allowed");
    assert_eq!(
        returned.texts,
        Some(vec!["```rust\na\n```".to_owned(), "plain text".to_owned()])
    );
}
