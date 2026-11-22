#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::call_helpers::metadata::MetadataSlots;
use betterlite_models::decisions::Json;

use crate::prometheus::failure::exception::LabelException;

use super::fallbacks::{log_failure_fallback_event, log_success_fallback_event};
use super::requests::FallbackRequest;
use super::support::{
    FakeCatalog, ProbeGauges, call, capture_logs, exception, expected_labels, inc_amount,
    json_object, labels_of, metric_of, now, update_of,
};

fn slots_with(pairs: &[(&str, &str)]) -> MetadataSlots {
    let entries: Vec<(&str, Json)> = pairs
        .iter()
        .map(|(name, value)| (*name, Json::Text((*value).to_owned())))
        .collect();
    MetadataSlots {
        metadata: Some(json_object(&entries)),
        litellm_metadata: None,
    }
}

fn request<'request>(
    model: Option<&'request str>,
    tags: &'request [String],
    metadata: &'request MetadataSlots,
    exception: &'request LabelException,
) -> FallbackRequest<'request> {
    FallbackRequest {
        original_model_group: "group-a",
        model,
        tags,
        metadata,
        exception,
        catalog: None,
        now: now(),
    }
}

fn expected_fallback_labels(status: &str, class: &str) -> BTreeMap<String, String> {
    expected_labels(
        "litellm_deployment_successful_fallbacks",
        &[
            ("requested_model", "group-a"),
            ("fallback_model", "fallback-1"),
            ("hashed_api_key", "hash-1"),
            ("api_key_alias", "alias-1"),
            ("team", "team-1"),
            ("team_alias", "team-alias-1"),
            ("exception_status", status),
            ("exception_class", class),
        ],
    )
}

#[test]
fn success_fallback_increments_the_successful_fallbacks_counter() {
    let gauges = ProbeGauges::new().with_exception_class("BadRequestError");
    let metadata = slots_with(&[
        ("user_api_key_hash", "hash-1"),
        ("user_api_key_alias", "alias-1"),
        ("user_api_key_team_id", "team-1"),
        ("user_api_key_team_alias", "team-alias-1"),
    ]);
    let status = exception(Some("400"), None);
    let tags: [String; 0] = [];
    log_success_fallback_event(
        &gauges,
        &request(Some("fallback-1"), &tags, &metadata, &status),
    )
    .unwrap();
    let events = gauges.updates();
    assert_eq!(events.len(), 1);
    assert_eq!(
        metric_of(&events[0]),
        "litellm_deployment_successful_fallbacks"
    );
    assert_eq!(update_of(&events[0]), inc_amount());
    assert_eq!(
        labels_of(&events[0]),
        &expected_fallback_labels("400", "BadRequestError")
    );
}

#[test]
fn failure_fallback_increments_the_failed_fallbacks_counter() {
    let gauges = ProbeGauges::new().with_exception_class("BadRequestError");
    let metadata = slots_with(&[
        ("user_api_key_hash", "hash-1"),
        ("user_api_key_alias", "alias-1"),
        ("user_api_key_team_id", "team-1"),
        ("user_api_key_team_alias", "team-alias-1"),
    ]);
    let status = exception(Some("400"), None);
    let tags: [String; 0] = [];
    log_failure_fallback_event(
        &gauges,
        &request(Some("fallback-1"), &tags, &metadata, &status),
    )
    .unwrap();
    let events = gauges.updates();
    assert_eq!(events.len(), 1);
    assert_eq!(metric_of(&events[0]), "litellm_deployment_failed_fallbacks");
    assert_eq!(update_of(&events[0]), inc_amount());
    assert_eq!(
        labels_of(&events[0])
            .get("fallback_model")
            .map(String::as_str),
        Some("fallback-1")
    );
}

#[test]
fn litellm_metadata_wins_over_metadata_when_both_are_present() {
    let gauges = ProbeGauges::new().with_exception_class("BadRequestError");
    let metadata = MetadataSlots {
        metadata: Some(json_object(&[(
            "user_api_key_alias",
            Json::Text("meta-alias".to_owned()),
        )])),
        litellm_metadata: Some(json_object(&[(
            "user_api_key_alias",
            Json::Text("litellm-alias".to_owned()),
        )])),
    };
    let status = exception(None, None);
    let tags: [String; 0] = [];
    log_success_fallback_event(&gauges, &request(None, &tags, &metadata, &status)).unwrap();
    let events = gauges.updates();
    assert_eq!(
        labels_of(&events[0])
            .get("api_key_alias")
            .map(String::as_str),
        Some("litellm-alias")
    );
}

#[test]
fn metadata_is_read_when_litellm_metadata_is_absent() {
    let gauges = ProbeGauges::new().with_exception_class("BadRequestError");
    let metadata = slots_with(&[("user_api_key_alias", "meta-alias")]);
    let status = exception(None, None);
    let tags: [String; 0] = [];
    log_success_fallback_event(&gauges, &request(None, &tags, &metadata, &status)).unwrap();
    let events = gauges.updates();
    assert_eq!(
        labels_of(&events[0])
            .get("api_key_alias")
            .map(String::as_str),
        Some("meta-alias")
    );
}

#[test]
fn an_exception_without_a_status_code_gives_the_text_none() {
    let gauges = ProbeGauges::new().with_exception_class("BadRequestError");
    let metadata = slots_with(&[("user_api_key_hash", "hash-1")]);
    let status = exception(None, None);
    let tags: [String; 0] = [];
    log_success_fallback_event(&gauges, &request(None, &tags, &metadata, &status)).unwrap();
    let events = gauges.updates();
    assert_eq!(
        labels_of(&events[0])
            .get("exception_status")
            .map(String::as_str),
        Some("None")
    );
}

#[test]
fn the_requested_model_is_bounded_against_the_catalog() {
    let gauges = ProbeGauges::new().with_exception_class("BadRequestError");
    let catalog = FakeCatalog::rejecting();
    let metadata = slots_with(&[]);
    let status = exception(None, None);
    let tags: [String; 0] = [];
    let fallback = FallbackRequest {
        original_model_group: "unknown-group",
        model: None,
        tags: &tags,
        metadata: &metadata,
        exception: &status,
        catalog: Some(&catalog),
        now: now(),
    };
    log_success_fallback_event(&gauges, &fallback).unwrap();
    let events = gauges.updates();
    assert_eq!(
        labels_of(&events[0])
            .get("requested_model")
            .map(String::as_str),
        Some("other")
    );
    assert_eq!(catalog.calls(), vec!["unknown-group".to_owned()]);
    assert!(
        gauges
            .calls()
            .contains(&call("bounded_model_label", "unknown-group"))
    );
}

#[test]
fn a_router_originated_group_passes_through_without_a_catalog() {
    let gauges = ProbeGauges::new().with_exception_class("BadRequestError");
    let metadata = slots_with(&[]);
    let status = exception(None, None);
    let tags: [String; 0] = [];
    log_success_fallback_event(&gauges, &request(None, &tags, &metadata, &status)).unwrap();
    let events = gauges.updates();
    assert_eq!(
        labels_of(&events[0])
            .get("requested_model")
            .map(String::as_str),
        Some("group-a")
    );
}

#[test]
fn the_success_event_logs_its_name_before_recording() {
    let gauges = ProbeGauges::new().with_exception_class("BadRequestError");
    let metadata = slots_with(&[]);
    let status = exception(None, None);
    let tags: [String; 0] = [];
    let (captured, _guard) = capture_logs(tracing::Level::DEBUG);
    log_success_fallback_event(&gauges, &request(None, &tags, &metadata, &status)).unwrap();
    assert!(
        captured
            .text()
            .contains("Prometheus: log_success_fallback_event, original_model_group: group-a")
    );
}

#[test]
fn the_failure_event_logs_its_name_before_recording() {
    let gauges = ProbeGauges::new().with_exception_class("BadRequestError");
    let metadata = slots_with(&[]);
    let status = exception(None, None);
    let tags: [String; 0] = [];
    let (captured, _guard) = capture_logs(tracing::Level::DEBUG);
    log_failure_fallback_event(&gauges, &request(None, &tags, &metadata, &status)).unwrap();
    assert!(
        captured
            .text()
            .contains("Prometheus: log_failure_fallback_event, original_model_group: group-a")
    );
}
