#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::TRUSTED_PILLAR_RESPONSE_HEADERS_METADATA_KEY;
use super::error::PillarError;
use super::response::{ProtectResponse, process_pillar_response};
use super::settings::{FallbackAction, OnFlaggedAction, PillarSettings};
use crate::guardrail_base::failure::GuardrailFailure;

fn pillar_settings(action: OnFlaggedAction) -> PillarSettings {
    PillarSettings {
        api_key: None,
        api_base: String::from("https://pillar.test"),
        on_flagged_action: action,
        async_mode: None,
        persist_session: None,
        include_scanners: true,
        include_evidence: true,
        fallback_on_error: FallbackAction::Allow,
        timeout: std::time::Duration::from_secs(5),
    }
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn messages() -> GuardrailValue {
    GuardrailValue::Array(vec![GuardrailValue::Object(
        [("role".to_owned(), text("user"))].into_iter().collect(),
    )])
}

fn data(entries: &[(&str, GuardrailValue)]) -> BTreeMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn response(flagged: bool) -> ProtectResponse {
    ProtectResponse {
        flagged: Some(flagged),
        session_id: Some(String::from("session-9")),
        scanners: Some(BTreeMap::from([(
            "pii".to_owned(),
            GuardrailValue::Boolean(true),
        )])),
        evidence: Some(vec![GuardrailValue::Object(
            [("evidence".to_owned(), text("secret"))]
                .into_iter()
                .collect(),
        )]),
        masked_session_messages: None,
        other: BTreeMap::new(),
    }
}

fn bucket<'data>(
    data: &'data BTreeMap<String, GuardrailValue>,
    key: &str,
) -> &'data BTreeMap<String, GuardrailValue> {
    let Some(GuardrailValue::Object(fields)) = data.get(key) else {
        panic!("{key} holds an object");
    };
    fields
}

#[test]
fn monitor_mode_stores_the_flag_scanners_evidence_and_session() {
    let mut request = data(&[("messages", messages())]);
    process_pillar_response(
        &response(true),
        &mut request,
        &pillar_settings(OnFlaggedAction::Monitor),
    )
    .expect("monitor mode does not fail");
    let metadata = bucket(&request, "metadata");
    assert_eq!(
        metadata.get("pillar_flagged"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(metadata.get("pillar_session_id"), Some(&text("session-9")));
    assert_eq!(
        metadata.get("pillar_session_id_response"),
        Some(&text("session-9"))
    );
    assert!(matches!(
        metadata.get("pillar_scanners"),
        Some(GuardrailValue::Object(_))
    ));
    assert!(matches!(
        metadata.get("pillar_evidence"),
        Some(GuardrailValue::Array(_))
    ));
    let headers = bucket(metadata, "pillar_response_headers");
    assert_eq!(headers.get("x-pillar-flagged"), Some(&text("true")));
    assert_eq!(headers.get("x-pillar-session-id"), Some(&text("session-9")));
    assert!(headers.contains_key("x-pillar-scanners"));
    assert!(headers.contains_key("x-pillar-evidence"));
    assert_eq!(
        metadata.get(TRUSTED_PILLAR_RESPONSE_HEADERS_METADATA_KEY),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[test]
fn an_existing_pillar_session_id_is_not_replaced_from_the_response_but_the_response_copy_is() {
    let metadata = GuardrailValue::Object(
        [("pillar_session_id".to_owned(), text("sent-session"))]
            .into_iter()
            .collect(),
    );
    let mut request = data(&[("messages", messages()), ("metadata", metadata)]);
    process_pillar_response(
        &response(false),
        &mut request,
        &pillar_settings(OnFlaggedAction::Monitor),
    )
    .expect("monitor mode does not fail");
    let metadata = bucket(&request, "metadata");
    assert_eq!(
        metadata.get("pillar_session_id"),
        Some(&text("sent-session"))
    );
    assert_eq!(
        metadata.get("pillar_session_id_response"),
        Some(&text("session-9"))
    );
}

#[test]
fn block_mode_raises_the_400_detail_after_writing_the_metadata() {
    let mut request = data(&[("messages", messages())]);
    let error = process_pillar_response(
        &response(true),
        &mut request,
        &pillar_settings(OnFlaggedAction::Block),
    )
    .expect_err("block mode raises");
    let PillarError::Http(GuardrailFailure::HttpObject { status, detail }) = error else {
        panic!("the block is an HTTP 400");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&text("Blocked by Pillar Security Guardrail"))
    );
    assert_eq!(
        detail.get("detection_message"),
        Some(&text("Security threats detected"))
    );
    let Some(GuardrailValue::Object(pillar_response)) = detail.get("pillar_response") else {
        panic!("the detail carries the pillar response");
    };
    assert_eq!(pillar_response.get("session_id"), Some(&text("session-9")));
    assert!(pillar_response.contains_key("scanners"));
    assert!(pillar_response.contains_key("evidence"));
    let metadata = bucket(&request, "metadata");
    assert_eq!(
        metadata.get("pillar_flagged"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert!(!metadata.contains_key("pillar_response_headers"));
}

#[test]
fn mask_mode_replaces_the_request_messages() {
    let mut response = response(true);
    response.masked_session_messages = Some(vec![GuardrailValue::Object(
        [
            ("role".to_owned(), text("user")),
            ("content".to_owned(), text("[masked]")),
        ]
        .into_iter()
        .collect(),
    )]);
    let mut request = data(&[("messages", messages())]);
    process_pillar_response(
        &response,
        &mut request,
        &pillar_settings(OnFlaggedAction::Mask),
    )
    .expect("mask mode does not fail");
    let Some(GuardrailValue::Array(replaced)) = request.get("messages") else {
        panic!("messages stay a list");
    };
    assert_eq!(replaced.len(), 1);
    assert_eq!(
        replaced.first().and_then(|message| match message {
            GuardrailValue::Object(fields) => fields.get("content"),
            _ => None,
        }),
        Some(&text("[masked]"))
    );
}

#[test]
fn mask_mode_without_masked_messages_leaves_the_request_alone() {
    let mut request = data(&[("messages", messages())]);
    process_pillar_response(
        &response(true),
        &mut request,
        &pillar_settings(OnFlaggedAction::Mask),
    )
    .expect("mask mode does not fail");
    assert_eq!(request.get("messages"), Some(&messages()));
}

#[test]
fn an_unflagged_response_in_block_mode_still_passes() {
    let mut request = data(&[("messages", messages())]);
    process_pillar_response(
        &response(false),
        &mut request,
        &pillar_settings(OnFlaggedAction::Block),
    )
    .expect("an unflagged response passes");
    assert_eq!(
        bucket(&request, "metadata").get("pillar_flagged"),
        Some(&GuardrailValue::Boolean(false))
    );
}

#[test]
fn a_litellm_metadata_bucket_receives_the_entries_and_metadata_aliases_it() {
    let mut request = data(&[
        ("litellm_metadata", GuardrailValue::Object(BTreeMap::new())),
        ("messages", messages()),
    ]);
    process_pillar_response(
        &response(true),
        &mut request,
        &pillar_settings(OnFlaggedAction::Monitor),
    )
    .expect("monitor mode does not fail");
    let resolved = bucket(&request, "litellm_metadata");
    let alias = bucket(&request, "metadata");
    assert_eq!(
        alias.get("pillar_flagged"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(alias, resolved);
}

#[test]
fn an_existing_metadata_object_stays_separate_from_litellm_metadata() {
    let existing =
        GuardrailValue::Object([("caller".to_owned(), text("kept"))].into_iter().collect());
    let mut request = data(&[
        ("litellm_metadata", GuardrailValue::Object(BTreeMap::new())),
        ("metadata", existing),
        ("messages", messages()),
    ]);
    process_pillar_response(
        &response(true),
        &mut request,
        &pillar_settings(OnFlaggedAction::Monitor),
    )
    .expect("monitor mode does not fail");
    assert_eq!(
        bucket(&request, "metadata").get("caller"),
        Some(&text("kept"))
    );
    assert!(!bucket(&request, "metadata").contains_key("pillar_flagged"));
    assert_eq!(
        bucket(&request, "litellm_metadata").get("pillar_flagged"),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[test]
fn a_non_object_metadata_bucket_is_replaced() {
    let mut request = data(&[("messages", messages()), ("metadata", text("junk"))]);
    process_pillar_response(
        &response(false),
        &mut request,
        &pillar_settings(OnFlaggedAction::Monitor),
    )
    .expect("monitor mode does not fail");
    assert!(matches!(
        request.get("metadata"),
        Some(GuardrailValue::Object(_))
    ));
}

#[test]
fn excluded_scanners_and_evidence_set_no_keys_and_no_headers() {
    let mut settings = pillar_settings(OnFlaggedAction::Monitor);
    settings.include_scanners = false;
    settings.include_evidence = false;
    let mut request = data(&[("messages", messages())]);
    process_pillar_response(&response(false), &mut request, &settings)
        .expect("monitor mode does not fail");
    let metadata = bucket(&request, "metadata");
    assert!(!metadata.contains_key("pillar_scanners"));
    assert!(!metadata.contains_key("pillar_evidence"));
    let headers = bucket(metadata, "pillar_response_headers");
    assert!(!headers.contains_key("x-pillar-scanners"));
    assert!(!headers.contains_key("x-pillar-evidence"));
}

#[rstest]
#[case::absent_scanners_and_evidence(true, false)]
#[case::both_excluded(false, false)]
fn missing_fields_of_the_response_default_to_empty_collections(
    #[case] include: bool,
    #[case] expected_present: bool,
) {
    let mut settings = pillar_settings(OnFlaggedAction::Monitor);
    settings.include_scanners = include;
    settings.include_evidence = include;
    let response = ProtectResponse {
        flagged: Some(false),
        ..ProtectResponse::default()
    };
    let mut request = data(&[("messages", messages())]);
    process_pillar_response(&response, &mut request, &settings)
        .expect("monitor mode does not fail");
    let metadata = bucket(&request, "metadata");
    assert_eq!(metadata.contains_key("pillar_scanners"), expected_present);
    assert_eq!(metadata.contains_key("pillar_evidence"), expected_present);
    if expected_present {
        assert_eq!(
            metadata.get("pillar_scanners"),
            Some(&GuardrailValue::Object(BTreeMap::new()))
        );
        assert_eq!(
            metadata.get("pillar_evidence"),
            Some(&GuardrailValue::Array(Vec::new()))
        );
    }
}

#[test]
fn an_empty_response_leaves_the_request_untouched() {
    let mut request = data(&[("messages", messages())]);
    process_pillar_response(
        &ProtectResponse::default(),
        &mut request,
        &pillar_settings(OnFlaggedAction::Block),
    )
    .expect("an empty response returns early");
    assert_eq!(request.len(), 1);
    assert!(!request.contains_key("metadata"));
}

#[test]
fn an_unknown_only_response_is_processed() {
    let response = ProtectResponse {
        other: BTreeMap::from([("note".to_owned(), text("hi"))]),
        ..ProtectResponse::default()
    };
    let mut request = data(&[("messages", messages())]);
    process_pillar_response(
        &response,
        &mut request,
        &pillar_settings(OnFlaggedAction::Monitor),
    )
    .expect("an unknown-only body is processed");
    assert_eq!(
        bucket(&request, "metadata").get("pillar_flagged"),
        Some(&GuardrailValue::Boolean(false))
    );
}

#[test]
fn the_block_detail_is_an_index_map_in_litellm_key_order() {
    let mut request = data(&[("messages", messages())]);
    let error = process_pillar_response(
        &response(true),
        &mut request,
        &pillar_settings(OnFlaggedAction::Block),
    )
    .expect_err("block mode raises");
    let PillarError::Http(GuardrailFailure::HttpObject { detail, .. }) = error else {
        panic!("the block is an HTTP 400");
    };
    let keys: Vec<&String> = detail.keys().collect();
    assert_eq!(keys, ["error", "detection_message", "pillar_response"]);
}
