#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::resolution::ResolutionError;
use rstest::rstest;
use serde_json::json;

use super::error::PillarError;
use super::model_provider::ModelProviderResolver;
use super::payload::{ProtectPayload, prepare_payload};

struct FixedResolver;

impl ModelProviderResolver for FixedResolver {
    fn model_and_provider(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
        _api_base: Option<&str>,
        _api_key: Option<&str>,
    ) -> Result<(String, String), ResolutionError> {
        Ok((String::from("clean-model"), String::from("openai")))
    }
}

fn data(entries: &[(&str, GuardrailValue)]) -> BTreeMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn messages() -> GuardrailValue {
    GuardrailValue::Array(vec![GuardrailValue::Object(
        [
            ("content".to_owned(), text("hi")),
            ("role".to_owned(), text("user")),
        ]
        .into_iter()
        .collect(),
    )])
}

fn prepared(entries: &[(&str, GuardrailValue)]) -> ProtectPayload {
    prepare_payload(&data(entries), &FixedResolver).expect("the payload is valid")
}

#[test]
fn the_payload_carries_messages_tools_metadata_and_the_resolved_model() {
    let payload = prepared(&[
        ("messages", messages()),
        ("model", text("openai/gpt-4o")),
        ("tools", GuardrailValue::Array(Vec::new())),
    ]);
    assert_eq!(
        serde_json::to_value(&payload).expect("the payload serializes"),
        json!({
            "messages": [{"content": "hi", "role": "user"}],
            "tools": [],
            "metadata": {"source": "litellm", "version": "1.105.1"},
            "model": "clean-model",
            "provider": "openai",
        })
    );
}

#[test]
fn the_user_id_is_sent_when_the_request_has_one() {
    let payload = prepared(&[
        ("messages", messages()),
        ("model", text("gpt-4o")),
        ("user", text("end-user-7")),
    ]);
    assert_eq!(payload.user_id, Some(String::from("end-user-7")));
    let serialized = serde_json::to_value(&payload).expect("the payload serializes");
    assert_eq!(serialized.get("user_id"), Some(&json!("end-user-7")));
}

#[test]
fn the_session_id_comes_from_the_metadata_bucket() {
    let metadata = GuardrailValue::Object(
        [("pillar_session_id".to_owned(), text("session-1"))]
            .into_iter()
            .collect(),
    );
    let payload = prepared(&[
        ("messages", messages()),
        ("metadata", metadata),
        ("model", text("gpt-4o")),
    ]);
    assert_eq!(payload.session_id, Some(String::from("session-1")));
}

#[test]
fn the_session_id_reads_metadata_even_when_litellm_metadata_is_present() {
    let metadata = GuardrailValue::Object(
        [("pillar_session_id".to_owned(), text("session-1"))]
            .into_iter()
            .collect(),
    );
    let other = GuardrailValue::Object(
        [("pillar_session_id".to_owned(), text("ignored"))]
            .into_iter()
            .collect(),
    );
    let payload = prepared(&[
        ("litellm_metadata", other),
        ("messages", messages()),
        ("metadata", metadata),
        ("model", text("gpt-4o")),
    ]);
    assert_eq!(payload.session_id, Some(String::from("session-1")));
}

#[test]
fn absent_messages_and_tools_become_empty_lists() {
    let payload = prepared(&[("model", text("gpt-4o"))]);
    assert_eq!(payload.messages, GuardrailValue::Array(Vec::new()));
    assert_eq!(payload.tools, GuardrailValue::Array(Vec::new()));
    assert_eq!(payload.user_id, None);
    assert_eq!(payload.session_id, None);
    let serialized = serde_json::to_value(&payload).expect("the payload serializes");
    assert_eq!(serialized.get("messages"), Some(&json!([])));
    assert_eq!(serialized.get("tools"), Some(&json!([])));
    assert!(serialized.get("user_id").is_none());
    assert!(serialized.get("session_id").is_none());
}

#[rstest]
#[case::messages_not_a_list("messages", GuardrailValue::Text("hi".to_owned()))]
#[case::tools_not_a_list("tools", GuardrailValue::SignedInteger(1))]
#[case::user_not_a_string("user", GuardrailValue::Boolean(true))]
fn a_wrongly_typed_request_field_names_itself(
    #[case] field: &'static str,
    #[case] value: GuardrailValue,
) {
    let entries: Vec<(&str, GuardrailValue)> = vec![("model", text("gpt-4o")), (field, value)];
    let error = prepare_payload(&data(&entries), &FixedResolver)
        .expect_err("a wrongly typed field is rejected");
    assert!(matches!(error, PillarError::Field { field: named, .. } if named == field));
}

#[test]
fn a_metadata_field_that_is_not_an_object_is_rejected() {
    let error = prepare_payload(
        &data(&[("metadata", text("nope")), ("model", text("gpt-4o"))]),
        &FixedResolver,
    )
    .expect_err("a non-object metadata is rejected");
    assert!(matches!(
        error,
        PillarError::Field {
            field: "metadata",
            ..
        }
    ));
}

#[test]
fn a_non_string_session_id_is_rejected() {
    let metadata = GuardrailValue::Object(
        [(
            "pillar_session_id".to_owned(),
            GuardrailValue::SignedInteger(7),
        )]
        .into_iter()
        .collect(),
    );
    let error = prepare_payload(
        &data(&[("metadata", metadata), ("model", text("gpt-4o"))]),
        &FixedResolver,
    )
    .expect_err("a non-string session id is rejected");
    assert!(matches!(
        error,
        PillarError::Field {
            field: "pillar_session_id",
            ..
        }
    ));
}
