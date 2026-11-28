#![cfg(test)]

use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_translation::{BaseTranslation as _, TranslationCall};

use crate::guardrails::unified_guardrail::other_adapters::google_genai::GoogleGenAiTranslation;
use crate::guardrails::unified_guardrail::tests::support::{FakeGuardrail, no_call, object, value};

#[tokio::test]
async fn google_genai_masks_request_and_response_text() {
    let guardrail = FakeGuardrail::masking();
    let genai = GoogleGenAiTranslation(PhantomData);
    let mut data = object(
        r#"{"model": "gemini", "contents": [{"role": "user", "parts": [{"text": "secret"}]}]}"#,
    );
    genai
        .process_input_messages(&mut data, guardrail.as_ref(), None)
        .await
        .unwrap();
    assert_eq!(
        GuardrailValue::Object(data),
        value(
            r#"{"model": "gemini", "contents": [{"role": "user", "parts": [{"text": "[masked]"}]}]}"#
        )
    );
    let response = value(r#"{"candidates": [{"content": {"parts": [{"text": "reply"}]}}]}"#);
    let checked = genai
        .process_output_response(response, guardrail.as_ref(), no_call())
        .await
        .unwrap();
    assert_eq!(
        checked,
        value(r#"{"candidates": [{"content": {"parts": [{"text": "[masked]"}]}}]}"#)
    );
    let seen = guardrail.seen();
    assert_eq!(seen[0].input_type, GuardrailInputType::Request);
    assert_eq!(seen[1].input_type, GuardrailInputType::Response);
    assert_eq!(seen[1].inputs.texts, Some(vec![String::from("reply")]));
}

#[tokio::test]
async fn google_genai_response_scan_reads_the_model_from_the_request() {
    let guardrail = FakeGuardrail::masking();
    let mut request = object(r#"{"model": "gemini-pro"}"#);
    let user = object(r#"{"user_id": "u1"}"#);
    let call = TranslationCall {
        user_api_key_dict: Some(&user),
        request_data: Some(&mut request),
        logging_obj: None,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: false,
    };
    let response = value(r#"{"candidates": [{"content": {"parts": [{"text": "reply"}]}}]}"#);
    GoogleGenAiTranslation(PhantomData)
        .process_output_response(response, guardrail.as_ref(), call)
        .await
        .unwrap();
    assert_eq!(
        guardrail.seen()[0].inputs.model.as_deref(),
        Some("gemini-pro")
    );
}

#[tokio::test]
async fn google_genai_scans_the_stream_text_so_far() {
    let guardrail = FakeGuardrail::masking();
    let mut frames = vec![
        StreamItem::Json(value(
            r#"{"candidates": [{"content": {"parts": [{"text": "sec"}]}}]}"#,
        )),
        StreamItem::Json(value(
            r#"{"candidates": [{"content": {"parts": [{"text": "ret"}]}}]}"#,
        )),
    ];
    let before = frames.clone();
    GoogleGenAiTranslation(PhantomData)
        .process_streaming_response(&mut frames, guardrail.as_ref(), no_call())
        .await
        .unwrap();
    assert_eq!(frames, before);
    assert_eq!(guardrail.seen_texts(), vec![vec![String::from("secret")]]);
}

#[tokio::test]
async fn google_genai_stream_bytes_frames_are_scanned() {
    let guardrail = FakeGuardrail::masking();
    let mut frames = vec![StreamItem::Bytes(
        b"data: {\"candidates\": [{\"content\": {\"parts\": [{\"text\": \"sec\xffret\"}]}}]}\r\n"
            .to_vec(),
    )];
    let before = frames.clone();
    GoogleGenAiTranslation(PhantomData)
        .process_streaming_response(&mut frames, guardrail.as_ref(), no_call())
        .await
        .unwrap();
    assert_eq!(frames, before);
    assert_eq!(
        guardrail.seen_texts(),
        vec![vec![String::from("sec\u{fffd}ret")]]
    );
}

#[tokio::test]
async fn google_genai_leaves_a_response_that_is_not_an_object() {
    let guardrail = FakeGuardrail::masking();
    let response = value(r#"["not", "an", "object"]"#);
    let checked = GoogleGenAiTranslation(PhantomData)
        .process_output_response(response.clone(), guardrail.as_ref(), no_call())
        .await
        .unwrap();
    assert_eq!(checked, response);
    assert!(guardrail.seen().is_empty());
}
