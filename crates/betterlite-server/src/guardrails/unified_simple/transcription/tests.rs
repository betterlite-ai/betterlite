#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};

use super::{TranscriptionTranslation, TranscriptionTranslationError};
use crate::guardrails::unified_simple::tests::fake::{FakeGuardrail, Recorded, calls_of};

fn body(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).unwrap()
}

fn call(
    request_data: &mut BTreeMap<String, GuardrailValue>,
) -> TranslationCall<'_, GuardrailLogTarget> {
    TranslationCall {
        user_api_key_dict: None,
        request_data: Some(request_data),
        logging_obj: None,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: false,
    }
}

fn refusal() -> GuardrailFailure {
    GuardrailFailure::Http {
        status: 400,
        detail: "blocked".to_owned(),
    }
}

fn is_refusal(error: &TranscriptionTranslationError) -> bool {
    matches!(
        error,
        TranscriptionTranslationError::Guardrail(GuardrailFailure::Http { status: 400, detail })
            if detail == "blocked"
    )
}

#[tokio::test]
async fn adapter_input_leaves_the_request_unchanged_without_a_guardrail_call() {
    let (guardrail, calls) = FakeGuardrail::replying(&["safe"]).erase();
    let original = body(r#"{"file": "audio.mp3", "model": "whisper-1"}"#);
    let mut data = original.clone();
    TranscriptionTranslation
        .process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert!(calls_of(&calls).is_empty());
    assert_eq!(data, original);
}

#[tokio::test]
async fn adapter_output_text_is_scanned_and_rewritten() {
    let (guardrail, calls) = FakeGuardrail::replying(&["safe-transcript"]).erase();
    let response = value(r#"{"text": "raw transcript", "model": "whisper-1"}"#);
    let mut request_data = BTreeMap::new();
    let rewritten = TranscriptionTranslation
        .process_output_response(response.clone(), &guardrail, call(&mut request_data))
        .await
        .unwrap();
    let recorded: Vec<Recorded> = calls_of(&calls);
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        recorded[0].inputs.texts,
        Some(vec!["raw transcript".to_owned()])
    );
    assert_eq!(recorded[0].inputs.model, Some("whisper-1".to_owned()));
    assert_eq!(recorded[0].input_type, GuardrailInputType::Response);
    assert_eq!(
        rewritten,
        value(r#"{"text": "safe-transcript", "model": "whisper-1"}"#)
    );
    assert_eq!(request_data.get("response"), Some(&response));
}

#[tokio::test]
async fn adapter_output_missing_text_makes_no_guardrail_call() {
    let (guardrail, calls) = FakeGuardrail::replying(&["safe"]).erase();
    let response = value(r#"{"model": "whisper-1"}"#);
    let mut request_data = BTreeMap::new();
    let returned = TranscriptionTranslation
        .process_output_response(response.clone(), &guardrail, call(&mut request_data))
        .await
        .unwrap();
    assert!(calls_of(&calls).is_empty());
    assert_eq!(returned, response);
    assert!(request_data.is_empty());
}

#[tokio::test]
async fn adapter_output_failure_is_returned() {
    let (guardrail, calls) = FakeGuardrail::failing(refusal()).erase();
    let response = value(r#"{"text": "raw transcript"}"#);
    let mut request_data = BTreeMap::new();
    let error = TranscriptionTranslation
        .process_output_response(response, &guardrail, call(&mut request_data))
        .await
        .unwrap_err();
    assert!(is_refusal(&error));
    assert_eq!(calls_of(&calls).len(), 1);
}

#[tokio::test]
async fn defaults_stream_methods_flags_and_hooks_keep_the_trait_values() {
    let (guardrail, calls) = FakeGuardrail::replying(&["safe"]).erase();
    let mut responses = vec![value(r#"{"text": "raw transcript"}"#)];
    let original = responses.clone();
    TranscriptionTranslation
        .process_streaming_response(&mut responses, &guardrail, call(&mut BTreeMap::new()))
        .await
        .unwrap();
    assert_eq!(responses, original);
    assert_eq!(
        TranscriptionTranslation.get_streaming_scan_key(&responses),
        None
    );
    let failure = TranscriptionTranslationError::Guardrail(refusal());
    assert_eq!(
        TranscriptionTranslation.build_block_sse_chunks(&failure, true, Some(&responses)),
        None
    );
    assert_eq!(
        TranscriptionTranslation.build_stream_error_items(&failure, Some(&responses)),
        None
    );
    assert_eq!(
        TranscriptionTranslation.get_structured_messages(&BTreeMap::new()),
        None
    );
    assert!(
        TranscriptionTranslation
            .extract_request_tool_names(&BTreeMap::new())
            .is_empty()
    );
    let response = value(r#"{"text": "raw transcript"}"#);
    assert_eq!(
        TranscriptionTranslation.post_call_hook_response(response.clone()),
        response
    );
    const {
        assert!(!<TranscriptionTranslation as BaseTranslation>::DELIVERS_ENDED_STREAM_REWRITES);
        assert!(!<TranscriptionTranslation as BaseTranslation>::ASSEMBLES_STREAMED_RESPONSE);
    }
    assert!(calls_of(&calls).is_empty());
}
