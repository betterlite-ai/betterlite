#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::prompt_injection::PromptInjectionParams;
use betterlite_models::billing_text::FormattedPromptData;
use reqwest::StatusCode;
use rstest::rstest;

use super::hook_support::{
    ATTACK, REJECTED_MESSAGE, Scenario, chat_messages, params, text, threshold, timeout,
};
use crate::prompt_injection::error::PromptInjectionError;
use crate::prompt_injection::hook::{HookCallType, PreCallOutcome, PromptInjectionDetection};
use crate::prompt_injection::pool::HeuristicsPool;

fn detection(params: Option<PromptInjectionParams>) -> PromptInjectionDetection {
    PromptInjectionDetection::new(params, threshold(), HeuristicsPool::new(1, timeout()))
}

#[tokio::test]
async fn an_unsupported_call_type_passes_without_scanning() {
    let detection = detection(Some(params(Scenario::Heuristics)));
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection
        .pre_call(HookCallType::Unsupported, &request)
        .await
        .unwrap();
    assert_eq!(outcome, PreCallOutcome::Pass);
}

#[tokio::test]
async fn embeddings_are_scanned_as_an_empty_prompt() {
    let detection = detection(None);
    let list = chat_messages();
    let input = text(ATTACK);
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: Some(&input),
    };
    let outcome = detection
        .pre_call(HookCallType::Embeddings, &request)
        .await
        .unwrap();
    assert_eq!(outcome, PreCallOutcome::Pass);
}

#[tokio::test]
async fn params_none_always_scans() {
    let detection = detection(None);
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection.pre_call(HookCallType::Completion, &request).await;
    assert!(matches!(outcome, Err(PromptInjectionError::Rejected)));
}

#[tokio::test]
async fn params_without_heuristics_do_not_scan() {
    let detection = detection(Some(params(Scenario::AllFalse)));
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection
        .pre_call(HookCallType::Acompletion, &request)
        .await
        .unwrap();
    assert_eq!(outcome, PreCallOutcome::Pass);
}

#[tokio::test]
async fn vector_db_check_does_nothing() {
    let detection = detection(Some(params(Scenario::VectorDb)));
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection
        .pre_call(HookCallType::Completion, &request)
        .await
        .unwrap();
    assert_eq!(outcome, PreCallOutcome::Pass);
}

#[rstest]
#[case::acompletion(HookCallType::Acompletion)]
#[case::completion(HookCallType::Completion)]
#[case::text_completion(HookCallType::TextCompletion)]
#[case::image_generation(HookCallType::ImageGeneration)]
#[case::moderation(HookCallType::Moderation)]
#[case::audio_transcription(HookCallType::AudioTranscription)]
#[tokio::test]
async fn every_accepted_call_type_scans(#[case] call_type: HookCallType) {
    let detection = detection(None);
    let list = chat_messages();
    let prompt = text(ATTACK);
    let input = text(ATTACK);
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: Some(&prompt),
        input: Some(&input),
    };
    let outcome = detection.pre_call(call_type, &request).await;
    assert!(matches!(outcome, Err(PromptInjectionError::Rejected)));
}

#[tokio::test]
async fn reject_as_response_returns_the_rejection_message() {
    let detection = detection(Some(params(Scenario::HeuristicsRejectAsResponse)));
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection
        .pre_call(HookCallType::Completion, &request)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        PreCallOutcome::RejectedAsMessage(REJECTED_MESSAGE.to_owned())
    );
}

#[rstest]
#[case::reject_as_response_false(Scenario::Heuristics)]
#[case::reject_as_response_none(Scenario::HeuristicsRejectAsResponseNone)]
#[tokio::test]
async fn a_heuristics_hit_rejects_when_reject_as_response_is_not_true(#[case] scenario: Scenario) {
    let detection = detection(Some(params(scenario)));
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let error = detection
        .pre_call(HookCallType::Completion, &request)
        .await
        .expect_err("an attack with heuristics enabled is rejected");
    assert!(matches!(error, PromptInjectionError::Rejected));
    let rejection = error.rejection().expect("a rejection carries a status");
    assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_malformed_body_passes_the_request() {
    let detection = detection(None);
    let request = FormattedPromptData {
        messages: None,
        prompt: None,
        input: None,
    };
    let outcome = detection
        .pre_call(HookCallType::Acompletion, &request)
        .await
        .unwrap();
    assert_eq!(outcome, PreCallOutcome::Pass);
}

#[tokio::test]
async fn a_failed_heuristics_check_passes_the_request() {
    let pool = HeuristicsPool::new(1, timeout());
    Arc::clone(&pool.permits).close();
    let detection = PromptInjectionDetection::new(None, threshold(), pool);
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let outcome = detection
        .pre_call(HookCallType::Acompletion, &request)
        .await
        .unwrap();
    assert_eq!(outcome, PreCallOutcome::Pass);
}

#[tokio::test]
async fn the_rejection_is_a_400_with_litellms_detail() {
    let detection = detection(None);
    let list = chat_messages();
    let request = FormattedPromptData {
        messages: Some(&list),
        prompt: None,
        input: None,
    };
    let error = detection
        .pre_call(HookCallType::Completion, &request)
        .await
        .unwrap_err();
    let rejection = error.rejection().expect("a rejection carries a status");
    assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        serde_json::to_string(&rejection.detail).unwrap(),
        r#"{"error":"Rejected message. This is a prompt injection attack."}"#
    );
    assert_eq!(error.to_string(), REJECTED_MESSAGE);
}

#[test]
fn other_failures_have_no_rejection_detail() {
    assert!(PromptInjectionError::ModelListNotSet.rejection().is_none());
    assert!(
        PromptInjectionError::InvalidLlmApiName
            .rejection()
            .is_none()
    );
    assert!(PromptInjectionError::Judge.rejection().is_none());
    assert!(PromptInjectionError::HeuristicsClosed.rejection().is_none());
}
