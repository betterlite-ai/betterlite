#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};

use super::{CompletionTranslation, CompletionTranslationError};
use crate::guardrails::unified_simple::tests::fake::{FakeGuardrail, Recorded, calls_of};

type Adapter = CompletionTranslation<FakeGuardrail>;

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

fn is_refusal(error: &CompletionTranslationError) -> bool {
    matches!(
        error,
        CompletionTranslationError::Guardrail(GuardrailFailure::Http { status: 400, detail })
            if detail == "blocked"
    )
}

#[tokio::test]
async fn adapter_input_prompt_is_scanned_and_rewritten() {
    let fake = FakeGuardrail::replying(&["safe"]);
    let calls = fake.calls();
    let mut log_target = GuardrailLogTarget::default();
    let original = body(r#"{"prompt": "unsafe", "model": "gpt-3.5-turbo-instruct"}"#);
    let mut data = original.clone();
    Adapter::default()
        .process_input_messages(&mut data, &fake, Some(&mut log_target))
        .await
        .unwrap();
    let recorded: Vec<Recorded> = calls_of(&calls);
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].logged);
    assert_eq!(recorded[0].inputs.texts, Some(vec!["unsafe".to_owned()]));
    assert_eq!(
        recorded[0].inputs.model,
        Some("gpt-3.5-turbo-instruct".to_owned())
    );
    assert_eq!(recorded[0].input_type, GuardrailInputType::Request);
    assert_eq!(recorded[0].request_data, original);
    assert_eq!(data.get("prompt"), Some(&value(r#""safe""#)));
    assert_eq!(
        data.get("model"),
        Some(&value(r#""gpt-3.5-turbo-instruct""#))
    );
}

#[tokio::test]
async fn adapter_input_list_prompt_rewrites_each_text() {
    let fake = FakeGuardrail::replying(&["one-safe", "two-safe"]);
    let calls = fake.calls();
    let mut data = body(r#"{"prompt": ["one", "two"]}"#);
    Adapter::default()
        .process_input_messages(&mut data, &fake, None)
        .await
        .unwrap();
    let recorded: Vec<Recorded> = calls_of(&calls);
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        recorded[0].inputs.texts,
        Some(vec!["one".to_owned(), "two".to_owned()])
    );
    assert_eq!(
        data.get("prompt"),
        Some(&value(r#"["one-safe", "two-safe"]"#))
    );
}

#[tokio::test]
async fn adapter_input_missing_prompt_makes_no_guardrail_call() {
    let fake = FakeGuardrail::replying(&["safe"]);
    let calls = fake.calls();
    let original = body(r#"{"model": "gpt-3.5-turbo-instruct"}"#);
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
    let original = body(r#"{"prompt": "unsafe"}"#);
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
async fn adapter_output_choice_texts_are_rewritten() {
    let fake = FakeGuardrail::replying(&["safe-a", "safe-b"]);
    let calls = fake.calls();
    let mut log_target = GuardrailLogTarget::default();
    let response = value(
        r#"{"object": "text_completion", "model": "m", "choices": [{"text": "a"}, {"text": "b"}]}"#,
    );
    let mut request_data = BTreeMap::new();
    let rewritten = Adapter::default()
        .process_output_response(
            response.clone(),
            &fake,
            call(&mut request_data, Some(&mut log_target)),
        )
        .await
        .unwrap();
    let recorded: Vec<Recorded> = calls_of(&calls);
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].logged);
    assert_eq!(
        recorded[0].inputs.texts,
        Some(vec!["a".to_owned(), "b".to_owned()])
    );
    assert_eq!(recorded[0].inputs.model, Some("m".to_owned()));
    assert_eq!(recorded[0].input_type, GuardrailInputType::Response);
    assert_eq!(
        rewritten,
        value(
            r#"{"object": "text_completion", "model": "m", "choices": [{"text": "safe-a"}, {"text": "safe-b"}]}"#
        )
    );
    assert_eq!(request_data.get("response"), Some(&response));
}

#[tokio::test]
async fn adapter_output_forwards_the_user_key_metadata() {
    let fake = FakeGuardrail::replying(&["safe-a"]);
    let calls = fake.calls();
    let user_key = body(r#"{"user_id": "u1"}"#);
    let mut request_data = BTreeMap::new();
    let output_call = TranslationCall {
        user_api_key_dict: Some(&user_key),
        request_data: Some(&mut request_data),
        logging_obj: None,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: false,
    };
    let response = value(r#"{"object": "text_completion", "choices": [{"text": "a"}]}"#);
    Adapter::default()
        .process_output_response(response, &fake, output_call)
        .await
        .unwrap();
    assert_eq!(
        calls_of(&calls)[0].request_data.get("litellm_metadata"),
        Some(&value(r#"{"user_api_key_user_id": "u1"}"#))
    );
}

#[tokio::test]
async fn adapter_output_without_choices_makes_no_guardrail_call() {
    let fake = FakeGuardrail::replying(&["safe"]);
    let calls = fake.calls();
    let response = value(r#"{"object": "text_completion"}"#);
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
async fn adapter_output_failure_is_returned() {
    let fake = FakeGuardrail::failing(refusal());
    let calls = fake.calls();
    let response = value(r#"{"object": "text_completion", "choices": [{"text": "a"}]}"#);
    let mut request_data = BTreeMap::new();
    let error = Adapter::default()
        .process_output_response(response, &fake, call(&mut request_data, None))
        .await
        .unwrap_err();
    assert!(is_refusal(&error));
    assert_eq!(calls_of(&calls).len(), 1);
}

#[tokio::test]
async fn defaults_stream_methods_flags_and_hooks_keep_the_trait_values() {
    let fake = FakeGuardrail::replying(&["safe"]);
    let calls = fake.calls();
    let mut responses = vec![value(r#"{"object": "text_completion"}"#)];
    let original = responses.clone();
    Adapter::default()
        .process_streaming_response(&mut responses, &fake, call(&mut BTreeMap::new(), None))
        .await
        .unwrap();
    assert_eq!(responses, original);
    assert_eq!(Adapter::default().get_streaming_scan_key(&responses), None);
    let failure = CompletionTranslationError::Guardrail(refusal());
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
    let response = value(r#"{"object": "text_completion"}"#);
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
