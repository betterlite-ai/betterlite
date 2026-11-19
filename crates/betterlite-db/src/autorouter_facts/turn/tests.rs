#![cfg(test)]

use std::str::FromStr as _;

use betterlite_models::model_registration::JsonObject;
use chrono::NaiveDateTime;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::autorouter_facts::session::SESSION_ID_MAX_CHARS;
use crate::autorouter_turn_sql::transaction::{AutoRouterTurnTransaction, TurnFlag, TurnTime};

use super::{TurnPayload, turn_transaction};

const SUCCESS: &str = r#"{
    "status": "success",
    "model": "gpt-4o",
    "model_group": "fast",
    "api_key": "sk-1",
    "user": "user-1",
    "session_id": "session-1",
    "startTime": "2024-01-02T03:04:05Z",
    "prompt_tokens": 10,
    "completion_tokens": 5,
    "spend": 0.5
}"#;

const DECISION: &str =
    r#"{"routing_decision":{"router_model_name":"smart","router_type":"llm-classifier"}}"#;

fn payload(text: &str) -> TurnPayload {
    serde_json::from_str(text).unwrap()
}

fn metadata(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn build(payload_text: &str, metadata_text: &str) -> Option<AutoRouterTurnTransaction> {
    turn_transaction(
        &payload(payload_text),
        &metadata(metadata_text),
        dec("0.25"),
    )
    .unwrap()
}

fn record(payload_text: &str, metadata_text: &str) -> AutoRouterTurnTransaction {
    build(payload_text, metadata_text).expect("the turn reaches the rollup")
}

fn naive(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").unwrap()
}

#[rstest]
#[case::failure("failure")]
#[case::empty("")]
#[case::uppercase("SUCCESS")]
fn only_a_successful_request_reaches_the_rollup(#[case] status: &str) {
    let turn = record(SUCCESS, DECISION);
    assert_eq!(turn.api_key, "sk-1");
    assert_eq!(turn.user_id, "user-1");
    assert_eq!(turn.session_id, "session-1");
    assert_eq!(turn.router_name, "smart");
    assert_eq!(turn.router_type, "llm-classifier");
    assert_eq!(turn.model, "gpt-4o");
    assert_eq!(turn.total_tokens, 15);
    assert_eq!(turn.spend, dec("0.5"));
    assert_eq!(turn.saved_spend, dec("0.25"));
    assert_eq!(turn.classifier_cost, Decimal::ZERO);
    assert_eq!(turn.tier, None);
    assert_eq!(turn.baseline_model, None);
    assert_eq!(turn.savings_estimated_turns, 0);
    assert_eq!(turn.savings_estimated_actual_spend, Decimal::ZERO);
    assert_eq!(turn.savings_estimated_saved_spend, Decimal::ZERO);
    let text = SUCCESS.replace("\"success\"", &format!("\"{status}\""));
    assert!(build(&text, DECISION).is_none());
}

#[rstest]
#[case::text_origin(r#""shadow_eval""#, false)]
#[case::true_origin("true", false)]
#[case::number_origin("1", false)]
#[case::list_origin("[1]", false)]
#[case::object_origin(r#"{"a":1}"#, false)]
#[case::false_origin("false", true)]
#[case::zero_origin("0", true)]
#[case::zero_fraction_origin("0.0", true)]
#[case::empty_text_origin(r#""""#, true)]
#[case::null_origin("null", true)]
#[case::empty_list_origin("[]", true)]
#[case::empty_object_origin("{}", true)]
fn an_internal_call_origin_excludes_the_turn(#[case] origin: &str, #[case] counted: bool) {
    let text = format!(
        r#"{{"internal_call_origin":{origin},"routing_decision":{{"router_model_name":"smart"}}}}"#
    );
    assert_eq!(build(SUCCESS, &text).is_some(), counted);
}

#[rstest]
#[case::absent("{}")]
#[case::empty_object(r#"{"routing_decision":{}}"#)]
#[case::list(r#"{"routing_decision":[]}"#)]
#[case::text(r#"{"routing_decision":"smart"}"#)]
#[case::null(r#"{"routing_decision":null}"#)]
fn a_missing_or_empty_routing_decision_excludes_the_turn(#[case] text: &str) {
    assert!(build(SUCCESS, text).is_none());
}

#[test]
fn the_router_model_name_wins_over_the_model_group() {
    let turn = record(SUCCESS, DECISION);
    assert_eq!(turn.router_name, "smart");
}

#[rstest]
#[case::empty_text(r#""""#)]
#[case::null("null")]
#[case::zero("0")]
#[case::false_literal("false")]
#[case::empty_list("[]")]
#[case::empty_object("{}")]
fn a_falsey_router_model_name_falls_back_to_the_model_group(#[case] name: &str) {
    let text = format!(r#"{{"routing_decision":{{"router_model_name":{name}}}}}"#);
    assert_eq!(record(SUCCESS, &text).router_name, "fast");
}

#[rstest]
#[case::number("5")]
#[case::true_literal("true")]
#[case::list("[1]")]
#[case::object(r#"{"a":1}"#)]
fn a_truthy_non_string_router_name_excludes_the_turn(#[case] name: &str) {
    let text = format!(r#"{{"routing_decision":{{"router_model_name":{name}}}}}"#);
    assert!(build(SUCCESS, &text).is_none());
}

#[test]
fn a_turn_without_any_router_name_is_excluded() {
    let text = SUCCESS.replace(r#""model_group": "fast","#, "");
    assert!(build(&text, r#"{"routing_decision":{"router_type":"heuristic"}}"#).is_none());
    assert!(build(&text, r#"{"routing_decision":{"router_model_name":""}}"#).is_none());
}

#[test]
fn identity_needs_a_key_or_a_user() {
    let no_key = SUCCESS.replace(r#""api_key": "sk-1","#, "");
    assert_eq!(record(&no_key, DECISION).user_id, "user-1");
    let no_user = SUCCESS.replace(r#""user": "user-1","#, "");
    assert_eq!(record(&no_user, DECISION).api_key, "sk-1");
    let neither = no_key.replace(r#""user": "user-1","#, "");
    assert!(build(&neither, DECISION).is_none());
    let empty_both = SUCCESS
        .replace(r#""api_key": "sk-1""#, r#""api_key": """#)
        .replace(r#""user": "user-1""#, r#""user": """#);
    assert!(build(&empty_both, DECISION).is_none());
}

#[rstest]
#[case::session_id("session_id", "session-1")]
#[case::model("model", "gpt-4o")]
fn a_turn_without_a_session_or_a_model_is_excluded(#[case] key: &str, #[case] value: &str) {
    let missing = SUCCESS.replace(&format!("\"{key}\": \"{value}\","), "");
    assert!(build(&missing, DECISION).is_none());
    let empty = SUCCESS.replace(
        &format!("\"{key}\": \"{value}\""),
        &format!("\"{key}\": \"\""),
    );
    assert!(build(&empty, DECISION).is_none());
}

#[test]
fn an_unparsable_start_time_excludes_the_turn() {
    let bad = SUCCESS.replace("2024-01-02T03:04:05Z", "yesterday");
    assert!(build(&bad, DECISION).is_none());
    let missing = SUCCESS.replace(r#""startTime": "2024-01-02T03:04:05Z","#, "");
    assert!(build(&missing, DECISION).is_none());
}

#[test]
fn an_aware_start_time_is_stored_in_utc() {
    let aware = SUCCESS.replace("2024-01-02T03:04:05Z", "2024-01-02T05:04:05.500000+02:00");
    let TurnTime::Naive(turn_at) = record(&aware, DECISION).turn_at else {
        panic!("the rollup stores a naive UTC time");
    };
    assert_eq!(turn_at, naive("2024-01-02T03:04:05.500000"));
}

#[test]
fn an_oversized_session_id_is_digested_and_a_short_one_is_kept() {
    let long = SUCCESS.replace("session-1", &"a".repeat(SESSION_ID_MAX_CHARS + 1));
    assert_eq!(
        record(&long, DECISION).session_id,
        "sha256:e8d95cc2b4bc198c54b40bd214df958afb65f5e73d2c2eafe0593cf5c635c1f0"
    );
    let short = SUCCESS.replace("session-1", &"a".repeat(256));
    assert_eq!(record(&short, DECISION).session_id, "a".repeat(256));
}

#[test]
fn the_classifier_cost_is_folded_into_the_turn_spend() {
    let split = r#"{"routing_decision":{"router_model_name":"smart","classifier_cost":0.25}}"#;
    let turn = record(SUCCESS, split);
    assert_eq!(turn.spend, dec("0.75"));
    assert_eq!(turn.classifier_cost, dec("0.25"));
    let no_spend = SUCCESS.replace(r#""spend": 0.5"#, r#""spend": null"#);
    assert_eq!(record(&no_spend, split).spend, dec("0.25"));
}

#[rstest]
#[case::quoted_number(r#""0.25""#)]
#[case::true_literal("true")]
#[case::null("null")]
#[case::list("[]")]
#[case::object("{}")]
fn a_non_numeric_classifier_cost_reads_as_zero(#[case] cost: &str) {
    let text = format!(
        r#"{{"routing_decision":{{"router_model_name":"smart","classifier_cost":{cost}}}}}"#
    );
    let turn = record(SUCCESS, &text);
    assert_eq!(turn.classifier_cost, Decimal::ZERO);
    assert_eq!(turn.spend, dec("0.5"));
}

#[test]
fn the_token_total_adds_prompt_and_completion_tokens() {
    let no_completion = SUCCESS.replace(r#""completion_tokens": 5,"#, "");
    assert_eq!(record(&no_completion, DECISION).total_tokens, 10);
    let neither = SUCCESS
        .replace(r#""prompt_tokens": 10,"#, "")
        .replace(r#""completion_tokens": 5,"#, "");
    assert_eq!(record(&neither, DECISION).total_tokens, 0);
}

#[test]
fn an_exact_estimated_saving_travels_with_the_turn() {
    let estimated = r#"{
        "routing_decision": {"router_model_name": "smart", "savings_baseline_model": "gpt-4o"},
        "autorouter_savings_estimate": {"version": 2, "status": "estimated"},
        "autorouter_savings": 1.5
    }"#;
    let turn = record(SUCCESS, estimated);
    assert_eq!(turn.savings_estimated_turns, 1);
    assert_eq!(turn.savings_estimated_actual_spend, dec("0.5"));
    assert_eq!(turn.savings_estimated_saved_spend, dec("1.5"));
    assert_eq!(turn.baseline_model.as_deref(), Some("gpt-4o"));
}

#[rstest]
#[case::measured_status(r#"{"routing_decision":{"router_model_name":"smart"},"autorouter_savings_estimate":{"version":2,"status":"measured"},"autorouter_savings":1.5}"#)]
#[case::unsupported_version(r#"{"routing_decision":{"router_model_name":"smart"},"autorouter_savings_estimate":{"version":4,"status":"estimated"},"autorouter_savings":1.5}"#)]
#[case::fractional_version(r#"{"routing_decision":{"router_model_name":"smart"},"autorouter_savings_estimate":{"version":2.0,"status":"estimated"},"autorouter_savings":1.5}"#)]
#[case::no_savings_amount(r#"{"routing_decision":{"router_model_name":"smart"},"autorouter_savings_estimate":{"version":2,"status":"estimated"}}"#)]
#[case::no_estimate(
    r#"{"routing_decision":{"router_model_name":"smart"},"autorouter_savings":1.5}"#
)]
fn an_inexact_or_absent_estimate_records_no_estimated_saving(#[case] metadata_text: &str) {
    let turn = record(SUCCESS, metadata_text);
    assert_eq!(turn.savings_estimated_turns, 0);
    assert_eq!(turn.savings_estimated_actual_spend, Decimal::ZERO);
    assert_eq!(turn.savings_estimated_saved_spend, Decimal::ZERO);
}

#[rstest]
#[case::number("5")]
#[case::true_literal("true")]
#[case::list("[]")]
#[case::object("{}")]
fn the_tier_travels_only_when_it_is_a_non_empty_string(#[case] tier: &str) {
    let tiered = r#"{"routing_decision":{"router_model_name":"smart","tier":"premium"}}"#;
    assert_eq!(record(SUCCESS, tiered).tier.as_deref(), Some("premium"));
    let text = format!(r#"{{"routing_decision":{{"router_model_name":"smart","tier":{tier}}}}}"#);
    assert_eq!(record(SUCCESS, &text).tier, None);
    let empty = r#"{"routing_decision":{"router_model_name":"smart","tier":""}}"#;
    assert_eq!(record(SUCCESS, empty).tier, None);
}

#[test]
fn the_baseline_model_travels_only_when_it_is_a_non_empty_string() {
    let baselined =
        r#"{"routing_decision":{"router_model_name":"smart","savings_baseline_model":"gpt-4o"}}"#;
    assert_eq!(
        record(SUCCESS, baselined).baseline_model.as_deref(),
        Some("gpt-4o")
    );
    let empty = r#"{"routing_decision":{"router_model_name":"smart","savings_baseline_model":""}}"#;
    assert_eq!(record(SUCCESS, empty).baseline_model, None);
    let numeric =
        r#"{"routing_decision":{"router_model_name":"smart","savings_baseline_model":5}}"#;
    assert_eq!(record(SUCCESS, numeric).baseline_model, None);
}

#[test]
fn the_router_type_defaults_to_unknown() {
    let absent = r#"{"routing_decision":{"router_model_name":"smart"}}"#;
    assert_eq!(record(SUCCESS, absent).router_type, "unknown");
    let empty = r#"{"routing_decision":{"router_model_name":"smart","router_type":""}}"#;
    assert_eq!(record(SUCCESS, empty).router_type, "unknown");
    let named = r#"{"routing_decision":{"router_model_name":"smart","router_type":"heuristic"}}"#;
    assert_eq!(record(SUCCESS, named).router_type, "heuristic");
    let numeric = r#"{"routing_decision":{"router_model_name":"smart","router_type":5}}"#;
    assert_eq!(record(SUCCESS, numeric).router_type, "unknown");
}

#[test]
fn absent_usage_reads_as_uncovered_and_touched() {
    let turn = record(SUCCESS, DECISION);
    assert_eq!(turn.covered, TurnFlag(false));
    assert_eq!(turn.cache_hit, TurnFlag(false));
    assert_eq!(turn.cache_ttl_seconds, None);
    assert_eq!(turn.cache_touched, TurnFlag(true));
}

#[test]
fn cache_facts_come_from_the_usage_object() {
    let cached = r#"{
        "routing_decision": {"router_model_name": "smart"},
        "usage_object": {
            "cache_read_input_tokens": 12,
            "prompt_tokens_details": {"cache_creation_token_details": {"ephemeral_1h_input_tokens": 4}}
        }
    }"#;
    let turn = record(SUCCESS, cached);
    assert_eq!(turn.covered, TurnFlag(true));
    assert_eq!(turn.cache_hit, TurnFlag(true));
    assert_eq!(turn.cache_ttl_seconds, Some(3600));
    assert_eq!(turn.cache_touched, TurnFlag(true));
}

#[test]
fn a_covered_turn_without_cache_traffic_is_not_touched() {
    let quiet =
        r#"{"routing_decision":{"router_model_name":"smart"},"usage_object":{"prompt_tokens":3}}"#;
    let turn = record(SUCCESS, quiet);
    assert_eq!(turn.covered, TurnFlag(true));
    assert_eq!(turn.cache_hit, TurnFlag(false));
    assert_eq!(turn.cache_ttl_seconds, None);
    assert_eq!(turn.cache_touched, TurnFlag(false));
}

#[rstest]
#[case::list("[]")]
#[case::text(r#""usage""#)]
#[case::number("5")]
#[case::null("null")]
#[case::empty_object("{}")]
fn a_non_mapping_usage_object_reads_as_absent(#[case] usage: &str) {
    let text =
        format!(r#"{{"routing_decision":{{"router_model_name":"smart"}},"usage_object":{usage}}}"#);
    let turn = record(SUCCESS, &text);
    assert_eq!(turn.covered, TurnFlag(false));
    assert_eq!(turn.cache_touched, TurnFlag(true));
}
