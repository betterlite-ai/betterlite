#![cfg(test)]

use std::sync::PoisonError;

use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use indexmap::IndexMap;
use serde_json::json;

use super::support::{
    StubLookup, TIMEOUT_SECONDS, decoding_to, fixture, list_call, logging, managed_retrieve_call,
    retrieve_call,
};

#[test]
fn prepare_decodes_the_managed_id_before_the_config_lookup() {
    let mut fixture = fixture();
    fixture.decoder.answer = decoding_to("azure", Some("deployment-7"));
    fixture.lookup = StubLookup::carrying("azure");
    let mut call = managed_retrieve_call();
    call.api_base = Some("https://ai.example".to_owned());
    let endpoints = fixture.endpoints();
    endpoints
        .container_file_call(
            "retrieve_container_file",
            &call,
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    let decoder_calls = fixture.decoder.seen();
    assert_eq!(decoder_calls.len(), 1);
    assert_eq!(decoder_calls[0].0, "litellm-managed-abc");
    assert_eq!(decoder_calls[0].1, "openai");
    assert_eq!(
        decoder_calls[0].2.get("api_base"),
        Some(&json!("https://ai.example"))
    );
    assert_eq!(decoder_calls[0].2.get("container_id"), None);
    assert_eq!(fixture.lookup.asked(), vec![ProviderId::Azure]);
    let recorded = fixture.handler.seen();
    assert_eq!(recorded[0].config_label, "azure");
    assert_eq!(
        recorded[0].values.get("container_id"),
        Some(&ParamValue::Text("cntr_original".to_owned()))
    );
    assert_eq!(
        recorded[0].params.get("model_id"),
        Some(&json!("deployment-7"))
    );
}

#[test]
fn prepare_logs_the_resolved_provider_and_the_path_params() {
    let mut fixture = fixture();
    fixture.decoder.answer = decoding_to("azure", None);
    let mut call = managed_retrieve_call();
    call.after = Some("cfile_0".to_owned());
    call.limit = Some(10);
    call.order = Some("asc".to_owned());
    call.litellm_call_id = Some("call-9".to_owned());
    let logging = logging();
    let endpoints = fixture.endpoints();
    endpoints
        .container_file_call("retrieve_container_file", &call, &logging, TIMEOUT_SECONDS)
        .expect("the handler answers");
    let state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    let optional_params = IndexMap::from([
        (
            "container_id".to_owned(),
            Json::Text("cntr_original".to_owned()),
        ),
        ("file_id".to_owned(), Json::Text("cfile_1".to_owned())),
    ]);
    assert_eq!(state.optional_params, optional_params);
    let litellm_params = IndexMap::from([(
        "litellm_call_id".to_owned(),
        Json::Text("call-9".to_owned()),
    )]);
    assert_eq!(state.litellm_params, litellm_params);
    assert_eq!(state.model, "");
    assert_eq!(state.custom_llm_provider.as_deref(), Some("azure"));
    assert_eq!(state.call_type, "container_file_call");
}

#[test]
fn prepare_logs_a_null_call_id_when_the_caller_gave_none() {
    let fixture = fixture();
    let logging = logging();
    let endpoints = fixture.endpoints();
    endpoints
        .container_file_call(
            "list_container_files",
            &list_call(),
            &logging,
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    let state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
    let optional_params =
        IndexMap::from([("container_id".to_owned(), Json::Text("cntr_1".to_owned()))]);
    assert_eq!(state.optional_params, optional_params);
}

#[test]
fn prepare_skips_decoding_without_a_container_id() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = retrieve_call();
    call.container_id = None;
    endpoints
        .container_file_call(
            "retrieve_container_file",
            &call,
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    assert!(fixture.decoder.seen().is_empty());
    assert_eq!(fixture.lookup.asked(), vec![ProviderId::Openai]);
    let recorded = fixture.handler.seen();
    assert!(recorded[0].values.get("container_id").is_none());
    assert_eq!(
        recorded[0].values.get("file_id"),
        Some(&ParamValue::Text("cfile_1".to_owned()))
    );
}

#[test]
fn prepare_unknown_provider_fails_before_the_handler() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = list_call();
    call.container_id = None;
    call.custom_llm_provider = "bogus".to_owned();
    let failure = endpoints
        .container_file_call("list_container_files", &call, &logging(), TIMEOUT_SECONDS)
        .expect_err("the provider name is unknown");
    assert_eq!(
        failure.error.to_string(),
        "\"bogus\" is not a provider betterlite knows"
    );
    assert!(fixture.decoder.seen().is_empty());
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.seen().is_empty());
}

#[test]
fn prepare_unknown_decoded_provider_fails_after_decoding() {
    let mut fixture = fixture();
    fixture.decoder.answer = decoding_to("bogus", None);
    let endpoints = fixture.endpoints();
    let failure = endpoints
        .container_file_call(
            "retrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect_err("the decoded provider name is unknown");
    assert_eq!(
        failure.error.to_string(),
        "\"bogus\" is not a provider betterlite knows"
    );
    assert_eq!(fixture.decoder.seen().len(), 1);
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.seen().is_empty());
}

#[test]
fn prepare_missing_provider_config_fails_with_litellms_text() {
    let mut fixture = fixture();
    fixture.lookup = StubLookup::without_config();
    let endpoints = fixture.endpoints();
    let failure = endpoints
        .container_file_call(
            "list_container_files",
            &list_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect_err("the provider has no container config");
    assert_eq!(
        failure.error.to_string(),
        "Container provider config not found for: openai"
    );
    assert_eq!(fixture.decoder.seen().len(), 1);
    assert_eq!(fixture.lookup.asked(), vec![ProviderId::Openai]);
    assert!(fixture.handler.seen().is_empty());
}
