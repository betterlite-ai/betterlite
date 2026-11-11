#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use crate::fields::payload::StandardLoggingPayload;
use crate::s3_transport::option_value::OptionValue;
use crate::sqs_logging::health::HEALTH_CHECK_PAYLOAD;

use super::{EventError, build};
use crate::legacy_s3::settings::Settings;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn settings() -> Settings {
    Settings {
        bucket_name: Some("logs".to_owned()),
        path: String::new(),
        region_name: Some("us-east-1".to_owned()),
        endpoint_url: None,
        access_key_id: None,
        secret_access_key: None,
        session_token: None,
        encryption: None,
        use_team_prefix: false,
        log_prompts_only: None,
        preview_features: false,
    }
}

fn payload() -> StandardLoggingPayload {
    serde_json::from_str(HEALTH_CHECK_PAYLOAD).expect("the health-check payload parses")
}

fn empty() -> BTreeMap<String, String> {
    BTreeMap::new()
}

#[test]
fn builds_the_day_partitioned_key_and_download_name() {
    let mut configured = settings();
    configured.path = "team-logs/".to_owned();
    let object = build(&payload(), &configured, &empty()).expect("the object builds");
    assert_eq!(
        object.object_key,
        "team-logs/2009-02-13/time-23-31-30-000000_sqs-health-check.json"
    );
    assert_eq!(
        object.download_filename,
        "time-2009-02-13T23-31-30-000000_sqs-health-check.json"
    );
}

#[test]
fn prefixes_the_team_alias_only_when_the_preview_gate_is_open() {
    let mut configured = settings();
    configured.preview_features = true;
    configured.use_team_prefix = true;
    let mut standard = payload();
    standard.metadata.user_api_key_team_alias = Some("team".to_owned());
    let object = build(&standard, &configured, &empty()).expect("the object builds");
    assert_eq!(
        object.object_key,
        "team/2009-02-13/time-23-31-30-000000_sqs-health-check.json"
    );
}

#[test]
fn a_closed_preview_gate_keeps_the_key_unprefixed() {
    let mut configured = settings();
    configured.use_team_prefix = true;
    let mut standard = payload();
    standard.metadata.user_api_key_team_alias = Some("team".to_owned());
    let object = build(&standard, &configured, &empty()).expect("the object builds");
    assert_eq!(
        object.object_key,
        "2009-02-13/time-23-31-30-000000_sqs-health-check.json"
    );
}

#[test]
fn a_disabled_team_prefix_keeps_the_key_unprefixed() {
    let mut configured = settings();
    configured.preview_features = true;
    let mut standard = payload();
    standard.metadata.user_api_key_team_alias = Some("team".to_owned());
    let object = build(&standard, &configured, &empty()).expect("the object builds");
    assert_eq!(
        object.object_key,
        "2009-02-13/time-23-31-30-000000_sqs-health-check.json"
    );
}

#[test]
fn nulls_the_response_when_prompts_only_is_configured() {
    let mut configured = settings();
    configured.log_prompts_only = Some(OptionValue::Text("true".to_owned()));
    let mut standard = payload();
    standard.response = Some(Json::Text("secret".to_owned()));
    let object = build(&standard, &configured, &empty()).expect("the object builds");
    let Json::Object(fields) = serde_json::from_str::<Json>(&object.body).expect("the body parses")
    else {
        panic!("the body is an object");
    };
    assert_eq!(fields.get("response"), Some(&Json::Null));
    assert_eq!(
        fields.get("id"),
        Some(&Json::Text("sqs-health-check".to_owned()))
    );
}

#[test]
fn nulls_the_response_when_the_environment_opens_prompts_only() {
    let configured = settings();
    let variables = environment(&[("S3_LOG_PROMPTS_ONLY", "true")]);
    let mut standard = payload();
    standard.response = Some(Json::Text("secret".to_owned()));
    let object = build(&standard, &configured, &variables).expect("the object builds");
    let Json::Object(fields) = serde_json::from_str::<Json>(&object.body).expect("the body parses")
    else {
        panic!("the body is an object");
    };
    assert_eq!(fields.get("response"), Some(&Json::Null));
}

#[test]
fn keeps_the_response_when_prompts_only_is_off() {
    let configured = settings();
    let mut standard = payload();
    standard.response = Some(Json::Text("visible".to_owned()));
    let object = build(&standard, &configured, &empty()).expect("the object builds");
    let Json::Object(fields) = serde_json::from_str::<Json>(&object.body).expect("the body parses")
    else {
        panic!("the body is an object");
    };
    assert_eq!(
        fields.get("response"),
        Some(&Json::Text("visible".to_owned()))
    );
}

#[test]
fn rejects_a_start_time_outside_the_calendar_range() {
    let configured = settings();
    let mut standard = payload();
    standard.start_time = Decimal::MAX;
    let error = build(&standard, &configured, &empty()).expect_err("the time is unusable");
    assert!(matches!(error, EventError::Timestamp { .. }));
}
