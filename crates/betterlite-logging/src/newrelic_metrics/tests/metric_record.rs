use std::str::FromStr as _;

use rstest::rstest;
use rust_decimal::Decimal;

use super::support::payload;
use crate::fields::payload::records::StandardLoggingMetadata;
use crate::fields::status::LoggingStatus;
use crate::newrelic_metrics::records::metric_record_from_payload;

#[test]
fn record_fields_come_from_the_standard_logging_payload() {
    let record = metric_record_from_payload(&payload()).expect("a record");
    assert_eq!(record.team_id, "team-1");
    assert_eq!(record.team_alias, "Team One");
    assert_eq!(record.model_group, "production");
    assert_eq!(record.model, "gpt-4o");
    assert_eq!(record.custom_llm_provider, "openai");
    assert_eq!(record.status, "success");
    assert_eq!(record.response_cost, Decimal::new(5, 1));
    assert_eq!(record.prompt_tokens, 2);
    assert_eq!(record.completion_tokens, 3);
    assert_eq!(record.total_tokens, 5);
    assert_eq!(record.duration_ms.to_bits(), 500.0_f64.to_bits());
    assert_eq!(record.team_max_budget, Some(Decimal::new(100, 1)));
    assert_eq!(record.team_spend, Some(Decimal::new(25, 1)));
}

#[test]
fn missing_optional_fields_default_to_empty_strings() {
    let mut payload = payload();
    payload.model_group = None;
    payload.custom_llm_provider = None;
    let record = metric_record_from_payload(&payload).expect("a record");
    assert_eq!(record.model_group, "");
    assert_eq!(record.custom_llm_provider, "");
}

#[test]
fn team_id_falls_back_to_the_metadata_team_id() {
    let mut payload = payload();
    payload.metadata.user_api_key_team_id = None;
    payload.metadata.team_id = Some("team-fallback".to_owned());
    let record = metric_record_from_payload(&payload).expect("a record");
    assert_eq!(record.team_id, "team-fallback");
}

#[test]
fn team_alias_falls_back_to_the_metadata_team_alias() {
    let mut payload = payload();
    payload.metadata.user_api_key_team_alias = None;
    payload.metadata.team_alias = Some("Fallback Team".to_owned());
    let record = metric_record_from_payload(&payload).expect("a record");
    assert_eq!(record.team_alias, "Fallback Team");
}

#[test]
fn the_api_key_team_wins_when_both_team_fields_are_set() {
    let mut payload = payload();
    payload.metadata.user_api_key_team_id = Some("api-key-team".to_owned());
    payload.metadata.team_id = Some("metadata-team".to_owned());
    payload.metadata.user_api_key_team_alias = Some("API Key Team".to_owned());
    payload.metadata.team_alias = Some("Metadata Team".to_owned());
    let record = metric_record_from_payload(&payload).expect("a record");
    assert_eq!(record.team_id, "api-key-team");
    assert_eq!(record.team_alias, "API Key Team");
}

#[test]
fn missing_team_metadata_becomes_an_empty_team() {
    let mut payload = payload();
    payload.metadata = StandardLoggingMetadata::default();
    let record = metric_record_from_payload(&payload).expect("a record");
    assert_eq!(record.team_id, "");
    assert_eq!(record.team_alias, "");
    assert_eq!(record.team_max_budget, None);
    assert_eq!(record.team_spend, None);
}

#[test]
fn failure_status_stays_a_lowercase_string() {
    let mut payload = payload();
    payload.status = LoggingStatus::Failure;
    let record = metric_record_from_payload(&payload).expect("a record");
    assert_eq!(record.status, "failure");
}

#[rstest]
#[case::zero(0.0_f64, 0.0_f64)]
#[case::half_second(0.5_f64, 500.0_f64)]
#[case::one_and_a_half(1.5_f64, 1_500.0_f64)]
fn duration_converts_seconds_to_milliseconds(#[case] seconds: f64, #[case] millis: f64) {
    let mut payload = payload();
    payload.response_time = Decimal::from_f64_retain(seconds).expect("a decimal second");
    let record = metric_record_from_payload(&payload).expect("a record");
    assert_eq!(record.duration_ms.to_bits(), millis.to_bits());
}

#[test]
fn duration_rounds_like_python_float_multiplication() {
    let mut payload = payload();
    payload.response_time = Decimal::from_str("21.326182382018498").expect("a decimal second");
    let record = metric_record_from_payload(&payload).expect("a record");
    assert_eq!(
        record.duration_ms.to_bits(),
        21_326.182_382_018_498_f64.to_bits()
    );
}
