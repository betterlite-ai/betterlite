#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{stream_inputs, stream_request_data};
use crate::guardrails::chat_guardrail_translation::streaming::tests::support::{
    TWO_CHOICES, chunks, object, principal,
};

#[test]
fn stream_request_data_holds_the_chunks_and_the_key_metadata() {
    let stream = chunks(TWO_CHOICES);
    let request_data = stream_request_data(&stream, &principal());
    let sent: Vec<GuardrailValue> = stream.into_iter().map(GuardrailValue::Object).collect();
    assert_eq!(
        request_data.get("responses"),
        Some(&GuardrailValue::Array(sent))
    );
    let metadata = GuardrailValue::Object(object(
        r#"{"user_api_key_user_id": "user-1", "user_api_key_user_email": "user@example.com"}"#,
    ));
    assert_eq!(request_data.get("metadata"), Some(&metadata));
}

#[test]
fn stream_inputs_carry_the_texts_and_the_model_of_the_first_chunk() {
    let inputs = stream_inputs(vec!["a".to_owned()], &chunks(TWO_CHOICES));
    assert_eq!(inputs.texts, Some(vec!["a".to_owned()]));
    assert_eq!(inputs.model.as_deref(), Some("gpt-4o"));
    assert_eq!(inputs.images, None);
    assert_eq!(inputs.tool_calls, None);
    assert_eq!(inputs.stream_holdback_chars, None);
}

#[rstest]
#[case::no_chunks("[]")]
#[case::no_model(r#"[{"choices": []}]"#)]
#[case::empty_model(r#"[{"model": "", "choices": []}]"#)]
#[case::null_model(r#"[{"model": null, "choices": []}]"#)]
#[case::model_that_is_not_text(r#"[{"model": 5, "choices": []}]"#)]
#[case::only_a_later_chunk_names_it(r#"[{"choices": []}, {"model": "gpt-4o"}]"#)]
fn stream_inputs_have_no_model_unless_the_first_chunk_names_one(#[case] source: &str) {
    let inputs = stream_inputs(Vec::new(), &chunks(source));
    assert_eq!(inputs.model, None);
}
