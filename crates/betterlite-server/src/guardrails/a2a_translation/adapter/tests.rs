#![cfg(test)]

use std::collections::BTreeMap;
use std::marker::PhantomData;

use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};

use super::{A2A_CALL_TYPES, A2ATranslation};
use crate::guardrails::a2a_translation::A2aTranslationError;
use crate::guardrails::a2a_translation::tests::support::{FakeGuardrail, body, value};

type Translation = A2ATranslation<FakeGuardrail>;

fn flags<T>() -> (bool, bool)
where
    T: BaseTranslation,
{
    (
        T::DELIVERS_ENDED_STREAM_REWRITES,
        T::ASSEMBLES_STREAMED_RESPONSE,
    )
}

fn translation() -> Translation {
    A2ATranslation {
        guardrail: PhantomData,
    }
}

fn failure() -> A2aTranslationError {
    A2aTranslationError::Guardrail(GuardrailFailure::Other("blocked".to_owned()))
}

#[test]
fn adapter_table_lists_send_message_before_asend_message() {
    assert_eq!(
        A2A_CALL_TYPES,
        [CallType::SendMessage, CallType::AsendMessage]
    );
}

#[test]
fn adapter_flags_are_false() {
    assert_eq!(flags::<Translation>(), (false, false));
}

#[tokio::test]
async fn adapter_input_scans_the_request_through_the_handler() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut data =
        body(r#"{"params":{"message":{"role":"user","parts":[{"kind":"text","text":"hello"}]}}}"#);
    translation()
        .process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    let rendered = serde_json::to_string(&data).unwrap();
    assert!(rendered.contains("rewritten"), "{rendered}");
    assert_eq!(
        guardrail.calls()[0].inputs.texts,
        Some(vec!["hello".to_owned()])
    );
}

#[tokio::test]
async fn adapter_output_returns_the_rewritten_response() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let response = value(r#"{"result":{"parts":[{"kind":"text","text":"hello"}]}}"#);
    let mut request_data = BTreeMap::new();
    let call = TranslationCall {
        user_api_key_dict: None,
        request_data: Some(&mut request_data),
        logging_obj: None,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: false,
    };
    let returned = translation()
        .process_output_response(response, &guardrail, call)
        .await
        .unwrap();
    let rendered = serde_json::to_string(&returned).unwrap();
    assert!(rendered.contains("rewritten"), "{rendered}");
    assert!(request_data.contains_key("response"));
}

#[tokio::test]
async fn adapter_stream_rewrites_the_chunks() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut responses = vec![
        value(r#"{"result":{"parts":[{"kind":"text","text":"a"}]}}"#),
        value(r#"{"result":{"parts":[{"kind":"text","text":"b"}]}}"#),
    ];
    let call = TranslationCall {
        user_api_key_dict: None,
        request_data: None,
        logging_obj: None,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: false,
    };
    translation()
        .process_streaming_response(&mut responses, &guardrail, call)
        .await
        .unwrap();
    assert_eq!(
        guardrail.calls()[0].inputs.texts,
        Some(vec!["ab".to_owned()])
    );
    assert_eq!(
        serde_json::to_string(&responses).unwrap(),
        r#"[{"result":{"parts":[{"kind":"text","text":"rewritten"}]}},{"result":{"parts":[{"kind":"text","text":""}]}}]"#
    );
}

#[test]
fn adapter_scan_key_holds_the_combined_text() {
    let responses = vec![
        value(r#"{"result":{"parts":[{"kind":"text","text":"a"}]}}"#),
        value(r#"{"result":{"parts":[{"kind":"text","text":"b"}]}}"#),
    ];
    let key = translation().get_streaming_scan_key(&responses).unwrap();
    assert_eq!(key.texts, vec!["ab".to_owned()]);
    assert!(!key.has_nothing_to_scan());
}

#[test]
fn adapter_default_methods_keep_the_base_answers() {
    let translation = translation();
    let data = BTreeMap::new();
    let blocked = failure();
    assert_eq!(translation.get_structured_messages(&data), None);
    assert!(translation.extract_request_tool_names(&data).is_empty());
    assert!(
        translation
            .build_block_sse_chunks(&blocked, false, None)
            .is_none()
    );
    assert!(
        translation
            .build_stream_error_items(&blocked, None)
            .is_none()
    );
    let response = value(r#"{"result":{}}"#);
    assert_eq!(
        translation.post_call_hook_response(response.clone()),
        response
    );
}
