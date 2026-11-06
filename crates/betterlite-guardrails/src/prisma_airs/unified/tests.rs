#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::{read, read_dir};
use std::path::Path;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::prisma_airs::inputs::Inputs;
use crate::prisma_airs::request_data::{
    LitellmParams, LoggingObject, ModelCallDetails, RequestData,
};
use crate::prisma_airs::support::{Run, execute, inputs, logging, request_data};

const ALLOW: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1"}"#;

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn metadata(entries: &[(&str, &str)]) -> BTreeMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), text(value)))
        .collect()
}

fn logging_with_model(model: &str) -> LoggingObject {
    let mut logging_obj = logging();
    logging_obj.model = Some(model.to_owned());
    logging_obj
}

fn logging_with_metadata(entries: &[(&str, &str)]) -> LoggingObject {
    let mut logging_obj = logging();
    logging_obj.model_call_details = Some(ModelCallDetails {
        litellm_params: Some(LitellmParams {
            metadata: Some(metadata(entries)),
        }),
    });
    logging_obj
}

fn with_metadata(entries: &[(&str, &str)]) -> RequestData {
    let mut data = request_data();
    data.metadata = Some(metadata(entries));
    data
}

#[tokio::test]
async fn logging_metadata_is_merged_under_the_request_metadata() {
    let data = with_metadata(&[("profile_name", "request-profile"), ("request_only", "r")]);
    let logging_obj =
        logging_with_metadata(&[("profile_name", "logging-profile"), ("logging_only", "l")]);
    let outcome = execute(Run::new(ALLOW), inputs(&["hi"]), data, Some(&logging_obj)).await;
    outcome.result.as_ref().expect("allowed");
    let merged = outcome.request_data.metadata.as_ref().expect("metadata");
    assert_eq!(merged.get("profile_name"), Some(&text("request-profile")));
    assert_eq!(merged.get("logging_only"), Some(&text("l")));
    assert!(outcome.bodies[0].contains(r#""profile_name":"request-profile""#));
}

#[tokio::test]
async fn the_inputs_model_fills_a_missing_request_model() {
    let mut given = inputs(&["hi"]);
    given.model = Some("input-model".to_owned());
    let outcome = execute(Run::new(ALLOW), given, request_data(), None).await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(outcome.request_data.model.as_deref(), Some("input-model"));
    assert!(outcome.bodies[0].contains(r#""ai_model":"input-model""#));
}

#[tokio::test]
async fn the_logging_model_is_the_last_fallback() {
    let logging_obj = logging_with_model("logging-model");
    let outcome = execute(
        Run::new(ALLOW),
        inputs(&["hi"]),
        request_data(),
        Some(&logging_obj),
    )
    .await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(outcome.request_data.model.as_deref(), Some("logging-model"));
}

#[tokio::test]
async fn the_request_model_wins_over_the_inputs_model() {
    let mut data = request_data();
    data.model = Some("request-model".to_owned());
    let mut given = inputs(&["hi"]);
    given.model = Some("input-model".to_owned());
    let outcome = execute(Run::new(ALLOW), given, data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(outcome.request_data.model.as_deref(), Some("request-model"));
}

#[tokio::test]
async fn a_synthesized_call_id_is_the_scan_tr_id() {
    let outcome = execute(Run::new(ALLOW), inputs(&["hi"]), request_data(), None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(r#""tr_id":"00000000-0000-0000-0000-000000000000""#));
}

#[tokio::test]
async fn a_stored_call_id_is_reused_by_the_next_scan() {
    let mut first = execute(Run::new(ALLOW), inputs(&["one"]), request_data(), None).await;
    first.result.as_ref().expect("allowed");
    first.request_data.litellm_call_id = Some("stored-call-id".to_owned());
    let second = execute(Run::new(ALLOW), inputs(&["two"]), first.request_data, None).await;
    second.result.as_ref().expect("allowed");
    assert!(second.bodies[0].contains(r#""tr_id":"stored-call-id""#));
}

#[tokio::test]
async fn a_logging_call_id_is_the_scan_tr_id_without_being_stored() {
    let mut logging_obj = logging();
    logging_obj.litellm_call_id = Some("log-id".to_owned());
    let outcome = execute(
        Run::new(ALLOW),
        inputs(&["hi"]),
        request_data(),
        Some(&logging_obj),
    )
    .await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(r#""tr_id":"log-id""#));
    assert_eq!(outcome.request_data.litellm_call_id, None);
}

#[tokio::test]
async fn include_guardrail_metadata_never_reaches_the_scan() {
    let data = with_metadata(&[("include_guardrail_metadata", "true")]);
    let outcome = execute(Run::new(ALLOW), inputs(&["hi"]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(!outcome.bodies[0].contains("include_guardrail_metadata"));
}

#[tokio::test]
async fn a_non_text_metadata_value_is_not_forwarded() {
    let mut data = request_data();
    data.metadata = Some(BTreeMap::from([(
        "profile_name".to_owned(),
        GuardrailValue::SignedInteger(7),
    )]));
    let outcome = execute(Run::new(ALLOW), inputs(&["hi"]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(r#""profile_name":"config-profile""#));
}

#[test]
fn replays_the_prisma_unified_corpus() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/prisma_unified");
    let entries: Vec<_> = read_dir(folder).expect("corpus").collect();
    assert!(!entries.is_empty());
    assert!(entries.into_iter().all(|entry| {
        let bytes = read(entry.expect("entry").path()).expect("fixture");
        serde_json::from_slice::<Inputs>(&bytes).is_ok()
    }));
}
