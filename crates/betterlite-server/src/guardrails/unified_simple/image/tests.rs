#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};

use super::{ImageTranslation, ImageTranslationError};
use crate::guardrails::unified_simple::tests::fake::{FakeGuardrail, Recorded, calls_of};

type Adapter = ImageTranslation<FakeGuardrail>;

fn body(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).unwrap()
}

fn call<'request>(
    request_data: &'request mut BTreeMap<String, GuardrailValue>,
    logging_obj: Option<&'request mut GuardrailLogTarget>,
) -> TranslationCall<'request, GuardrailLogTarget> {
    TranslationCall {
        user_api_key_dict: None,
        request_data: Some(request_data),
        logging_obj,
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

fn is_refusal(error: &ImageTranslationError) -> bool {
    matches!(
        error,
        ImageTranslationError::Guardrail(GuardrailFailure::Http { status: 400, detail })
            if detail == "blocked"
    )
}

#[tokio::test]
async fn adapter_input_prompt_is_scanned_and_rewritten() {
    let fake = FakeGuardrail::replying(&["safe-prompt"]);
    let calls = fake.calls();
    let mut log_target = GuardrailLogTarget::default();
    let original = body(r#"{"prompt": "raw-prompt", "model": "dall-e-3"}"#);
    let mut data = original.clone();
    Adapter::default()
        .process_input_messages(&mut data, &fake, Some(&mut log_target))
        .await
        .unwrap();
    let recorded: Vec<Recorded> = calls_of(&calls);
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].logged);
    assert_eq!(
        recorded[0].inputs.texts,
        Some(vec!["raw-prompt".to_owned()])
    );
    assert_eq!(recorded[0].inputs.model, Some("dall-e-3".to_owned()));
    assert_eq!(recorded[0].input_type, GuardrailInputType::Request);
    assert_eq!(recorded[0].request_data, original);
    assert_eq!(data.get("prompt"), Some(&value(r#""safe-prompt""#)));
}

#[tokio::test]
async fn adapter_input_missing_prompt_makes_no_guardrail_call() {
    let fake = FakeGuardrail::replying(&["safe"]);
    let calls = fake.calls();
    let original = body(r#"{"model": "dall-e-3"}"#);
    let mut data = original.clone();
    Adapter::default()
        .process_input_messages(&mut data, &fake, None)
        .await
        .unwrap();
    assert!(calls_of(&calls).is_empty());
    assert_eq!(data, original);
}

#[tokio::test]
async fn adapter_input_failure_is_returned() {
    let fake = FakeGuardrail::failing(refusal());
    let calls = fake.calls();
    let original = body(r#"{"prompt": "raw-prompt"}"#);
    let mut data = original.clone();
    let error = Adapter::default()
        .process_input_messages(&mut data, &fake, None)
        .await
        .unwrap_err();
    assert!(is_refusal(&error));
    assert_eq!(calls_of(&calls).len(), 1);
    assert_eq!(data, original);
}

#[tokio::test]
async fn adapter_output_returns_the_response_without_a_guardrail_call() {
    let fake = FakeGuardrail::replying(&["safe"]);
    let calls = fake.calls();
    let response = value(r#"{"created": 1, "data": [{"b64_json": "AA=="}]}"#);
    let mut request_data = BTreeMap::new();
    let returned = Adapter::default()
        .process_output_response(response.clone(), &fake, call(&mut request_data, None))
        .await
        .unwrap();
    assert!(calls_of(&calls).is_empty());
    assert_eq!(returned, response);
    assert!(request_data.is_empty());
}

#[tokio::test]
async fn defaults_stream_methods_flags_and_hooks_keep_the_trait_values() {
    let fake = FakeGuardrail::replying(&["safe"]);
    let calls = fake.calls();
    let mut responses = vec![value(r#"{"created": 1}"#)];
    let original = responses.clone();
    Adapter::default()
        .process_streaming_response(&mut responses, &fake, call(&mut BTreeMap::new(), None))
        .await
        .unwrap();
    assert_eq!(responses, original);
    assert_eq!(Adapter::default().get_streaming_scan_key(&responses), None);
    let failure = ImageTranslationError::Guardrail(refusal());
    assert_eq!(
        Adapter::default().build_block_sse_chunks(&failure, true, Some(&responses)),
        None
    );
    assert_eq!(
        Adapter::default().build_stream_error_items(&failure, Some(&responses)),
        None
    );
    assert_eq!(
        Adapter::default().get_structured_messages(&BTreeMap::new()),
        None
    );
    assert!(
        Adapter::default()
            .extract_request_tool_names(&BTreeMap::new())
            .is_empty()
    );
    let response = value(r#"{"created": 1}"#);
    assert_eq!(
        Adapter::default().post_call_hook_response(response.clone()),
        response
    );
    const {
        assert!(!<Adapter as BaseTranslation>::DELIVERS_ENDED_STREAM_REWRITES);
        assert!(!<Adapter as BaseTranslation>::ASSEMBLES_STREAMED_RESPONSE);
    }
    assert!(calls_of(&calls).is_empty());
}
