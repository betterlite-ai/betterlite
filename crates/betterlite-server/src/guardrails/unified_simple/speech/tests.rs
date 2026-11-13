#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};

use super::{SpeechTranslation, SpeechTranslationError};
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

fn is_refusal(error: &SpeechTranslationError) -> bool {
    matches!(
        error,
        SpeechTranslationError::Guardrail(GuardrailFailure::Http { status: 400, detail })
            if detail == "blocked"
    )
}

#[tokio::test]
async fn adapter_input_text_is_scanned_and_rewritten() {
    let (guardrail, calls) = FakeGuardrail::replying(&["safe-speech"]).erase();
    let original = body(r#"{"input": "read this", "model": "tts-1", "voice": "alloy"}"#);
    let mut data = original.clone();
    SpeechTranslation
        .process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    let recorded: Vec<Recorded> = calls_of(&calls);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].inputs.texts, Some(vec!["read this".to_owned()]));
    assert_eq!(recorded[0].inputs.model, Some("tts-1".to_owned()));
    assert_eq!(recorded[0].input_type, GuardrailInputType::Request);
    assert_eq!(recorded[0].request_data, original);
    assert_eq!(data.get("input"), Some(&value(r#""safe-speech""#)));
    assert_eq!(data.get("voice"), Some(&value(r#""alloy""#)));
}

#[tokio::test]
async fn adapter_input_missing_text_makes_no_guardrail_call() {
    let (guardrail, calls) = FakeGuardrail::replying(&["safe"]).erase();
    let original = body(r#"{"model": "tts-1"}"#);
    let mut data = original.clone();
    SpeechTranslation
        .process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert!(calls_of(&calls).is_empty());
    assert_eq!(data, original);
}

#[tokio::test]
async fn adapter_input_failure_is_returned() {
    let (guardrail, calls) = FakeGuardrail::failing(refusal()).erase();
    let original = body(r#"{"input": "read this"}"#);
    let mut data = original.clone();
    let error = SpeechTranslation
        .process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap_err();
    assert!(is_refusal(&error));
    assert_eq!(calls_of(&calls).len(), 1);
    assert_eq!(data, original);
}

#[tokio::test]
async fn adapter_output_returns_the_response_without_a_guardrail_call() {
    let (guardrail, calls) = FakeGuardrail::replying(&["safe"]).erase();
    let response = value(r#"{"audio": "YXVkaW8="}"#);
    let mut request_data = BTreeMap::new();
    let returned = SpeechTranslation
        .process_output_response(response.clone(), &guardrail, call(&mut request_data))
        .await
        .unwrap();
    assert!(calls_of(&calls).is_empty());
    assert_eq!(returned, response);
    assert!(request_data.is_empty());
}

#[tokio::test]
async fn defaults_stream_methods_flags_and_hooks_keep_the_trait_values() {
    let (guardrail, calls) = FakeGuardrail::replying(&["safe"]).erase();
    let mut responses = vec![value(r#"{"audio": "YXVkaW8="}"#)];
    let original = responses.clone();
    SpeechTranslation
        .process_streaming_response(&mut responses, &guardrail, call(&mut BTreeMap::new()))
        .await
        .unwrap();
    assert_eq!(responses, original);
    assert_eq!(SpeechTranslation.get_streaming_scan_key(&responses), None);
    let failure = SpeechTranslationError::Guardrail(refusal());
    assert_eq!(
        SpeechTranslation.build_block_sse_chunks(&failure, true, Some(&responses)),
        None
    );
    assert_eq!(
        SpeechTranslation.build_stream_error_items(&failure, None),
        None
    );
    assert_eq!(
        SpeechTranslation.get_structured_messages(&BTreeMap::new()),
        None
    );
    assert!(
        SpeechTranslation
            .extract_request_tool_names(&BTreeMap::new())
            .is_empty()
    );
    let response = value(r#"{"audio": "YXVkaW8="}"#);
    assert_eq!(
        SpeechTranslation.post_call_hook_response(response.clone()),
        response
    );
    const {
        assert!(!<SpeechTranslation as BaseTranslation>::DELIVERS_ENDED_STREAM_REWRITES);
        assert!(!<SpeechTranslation as BaseTranslation>::ASSEMBLES_STREAMED_RESPONSE);
    }
    assert!(calls_of(&calls).is_empty());
}
