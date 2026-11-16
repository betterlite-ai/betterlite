#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::model_armor::hooks::logging::process_response;
use crate::model_armor::hooks::tests::support::{
    RECORDS, Recorder, armor, armor_with, data, field, prompt_data, records, settings_for, text,
    value,
};

#[tokio::test]
async fn logging_records_the_stored_response_and_the_intervened_status() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let mut request =
        data(r#"{"metadata":{"_model_armor_response":{"x":1},"_model_armor_status":"blocked"}}"#);
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    process_response(&hooks, &mut request, &mut recorder.context()).expect("record");
    let entries = records(&request, "metadata");
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(field(entry, "guardrail_response"), value(r#"{"x":1}"#));
    assert_eq!(
        field(entry, "guardrail_status"),
        text("guardrail_intervened")
    );
    assert_eq!(field(entry, "guardrail_provider"), GuardrailValue::Null);
    assert_eq!(field(entry, "guardrail_mode"), text("post_call"));
    assert_eq!(field(entry, "start_time"), GuardrailValue::Decimal(1.5));
    assert_eq!(field(entry, "end_time"), GuardrailValue::Decimal(100.0));
    assert_eq!(field(entry, "duration"), GuardrailValue::Decimal(98.5));
    assert!(recorder.invocation.self_recorded);
}

#[tokio::test]
async fn logging_defaults_to_an_empty_response_and_success() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    process_response(&hooks, &mut request, &mut recorder.context()).expect("record");
    let entries = records(&request, "metadata");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        field(&entries[0], "guardrail_response"),
        GuardrailValue::Object(BTreeMap::new())
    );
    assert_eq!(field(&entries[0], "guardrail_status"), text("success"));
    assert_eq!(field(&entries[0], "guardrail_mode"), text("pre_call"));
}

#[tokio::test]
async fn logging_reads_the_litellm_metadata_bucket_first() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let mut request = data(
        r#"{"metadata":{"_model_armor_response":"old","_model_armor_status":"success"},"litellm_metadata":{"_model_armor_response":"new","_model_armor_status":"blocked"}}"#,
    );
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    process_response(&hooks, &mut request, &mut recorder.context()).expect("record");
    let entries = records(&request, "litellm_metadata");
    assert_eq!(entries.len(), 1);
    assert_eq!(field(&entries[0], "guardrail_response"), text("new"));
    assert_eq!(
        field(&entries[0], "guardrail_status"),
        text("guardrail_intervened")
    );
    assert!(records(&request, "metadata").is_empty());
}

#[tokio::test]
async fn logging_without_an_event_type_records_the_configured_mode() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(None);
    process_response(&hooks, &mut request, &mut recorder.context()).expect("record");
    let entries = records(&request, "metadata");
    assert_eq!(
        field(&entries[0], "guardrail_mode"),
        value(r#"["pre_call","during_call","post_call"]"#)
    );
}

#[tokio::test]
async fn logging_records_the_invocation_event_type_over_the_configured_hook() {
    let fixture = armor_with(Vec::new(), settings_for(&["during_call"]), false).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    process_response(&hooks, &mut request, &mut recorder.context()).expect("record");
    let entries = records(&request, "metadata");
    assert_eq!(field(&entries[0], "guardrail_mode"), text("post_call"));
}

#[tokio::test]
async fn logging_syncs_the_records_into_the_log_target() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    process_response(&hooks, &mut request, &mut recorder.context()).expect("record");
    let target = recorder.log_target.as_ref().expect("target");
    let params = target.litellm_params.as_ref().expect("params");
    let Some(GuardrailValue::Object(metadata)) = params.get("metadata") else {
        panic!("no metadata in {params:?}");
    };
    match metadata.get(RECORDS) {
        Some(GuardrailValue::Array(items)) => assert_eq!(items.len(), 1),
        other => panic!("expected the guardrail records, got {other:?}"),
    }
}

#[tokio::test]
async fn logging_without_a_log_target_still_records() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    recorder.log_target = None;
    process_response(&hooks, &mut request, &mut recorder.context()).expect("record");
    assert_eq!(records(&request, "metadata").len(), 1);
}

#[tokio::test]
async fn logging_appends_to_the_existing_records() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let mut request = data(
        r#"{"metadata":{"standard_logging_guardrail_information":[{"guardrail_name":"first"}]}}"#,
    );
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    process_response(&hooks, &mut request, &mut recorder.context()).expect("record");
    let entries = records(&request, "metadata");
    assert_eq!(entries.len(), 2);
    assert_eq!(field(&entries[0], "guardrail_name"), text("first"));
    assert_eq!(field(&entries[1], "guardrail_name"), text("armor"));
}
