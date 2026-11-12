#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::num::NonZeroU64;

use rstest::rstest;
use serde::de::Visitor as _;
use serde::de::value::{BytesDeserializer, Error as ValueError};
use serde::{Deserialize as _, Serialize};

use super::Settings;
use super::base::{BaseLitellmParams, OnSensitiveData, UnreachableFallback};
use super::decision_check::{DecisionCheckAction, DecisionModelCheck, GuardrailChecks};
use super::decode::decode_litellm_params;
use super::error::GuardrailParamsError;
use super::keys::{BASE_KEYS, BOOL_KEYS, NUMBER_KEYS, PARAM_KEYS};
use super::mode::{LitellmParamsMode, Mode, ModeTarget};
use super::opaque::{ExactValue, OpaqueParams, ValueVisitor};
use super::params::LitellmParams;
use super::records::Guardrail;
use crate::loader::ConfigLoader;
use crate::sections::guardrail_identity::item::GuardrailValue;

const ENTRY: &str = r#""guardrail": "custom", "mode": "pre_call""#;

fn opaque(json: &str) -> OpaqueParams {
    serde_json::from_str(json).unwrap()
}

fn entry(fields: &str) -> String {
    format!("{{{ENTRY}, {fields}}}")
}

fn base(json: &str) -> Result<BaseLitellmParams, GuardrailParamsError> {
    BaseLitellmParams::try_from(opaque(json))
}

fn params(json: &str) -> Result<LitellmParams, GuardrailParamsError> {
    LitellmParams::try_from(opaque(json))
}

fn dump<T>(value: &T) -> BTreeMap<String, GuardrailValue>
where
    T: Serialize,
{
    let bytes = serde_json::to_vec(value).unwrap();
    serde_json::from_slice::<OpaqueParams>(&bytes).unwrap().0
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn refused_field<T>(result: Result<T, GuardrailParamsError>) -> String
where
    T: Debug,
{
    let error = result.unwrap_err();
    let GuardrailParamsError::InvalidField { field } = &error else {
        panic!("expected InvalidField, got {error:?}");
    };
    field.clone()
}

fn load(source: &str, environment: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(source).unwrap();
    config.sections.guardrail_params
}

fn minimal(extra: &str) -> String {
    format!(
        "guardrails:\n  - guardrail_name: safety\n    litellm_params:\n      guardrail: custom\n      mode: pre_call\n{extra}"
    )
}

#[test]
fn defaults_follow_litellm() {
    let parsed = base("{}").unwrap();
    assert!(parsed.fields.fail_on_error);
    assert!(parsed.fields.sanitize_error_detail);
    assert!(parsed.handling_flags.sticky_session_routing);
    assert!(!parsed.handling_flags.skip_unscannable_attachments);
    assert!(!parsed.scan_flags.latest_role_message_only);
    assert!(!parsed.scan_flags.only_scan_new_messages);
    assert_eq!(
        parsed.fields.unreachable_fallback,
        UnreachableFallback::FailClosed
    );
    assert!(parsed.extra.0.is_empty());
}

#[test]
fn defaults_dump_writes_null_for_unset_fields() {
    let written = dump(&base("{}").unwrap());
    assert_eq!(written["api_key"], GuardrailValue::Null);
    assert_eq!(written["timeout"], GuardrailValue::Null);
    assert_eq!(written["default_on"], GuardrailValue::Null);
    assert_eq!(written["patterns"], GuardrailValue::Null);
    assert_eq!(written["fail_on_error"], GuardrailValue::Boolean(true));
    assert_eq!(
        written["sticky_session_routing"],
        GuardrailValue::Boolean(true)
    );
    assert_eq!(
        written["skip_unscannable_attachments"],
        GuardrailValue::Boolean(false)
    );
    assert_eq!(written["unreachable_fallback"], text("fail_closed"));
}

#[test]
fn defaults_apply_to_explicit_null() {
    let parsed = base(
        r#"{"fail_on_error": null, "sanitize_error_detail": null, "sticky_session_routing": null, "unreachable_fallback": null}"#,
    )
    .unwrap();
    assert!(parsed.fields.fail_on_error);
    assert!(parsed.fields.sanitize_error_detail);
    assert!(parsed.handling_flags.sticky_session_routing);
    assert_eq!(
        parsed.fields.unreachable_fallback,
        UnreachableFallback::FailClosed
    );
}

#[test]
fn keys_match_the_serialized_default() {
    let written: Vec<String> = dump(&base("{}").unwrap()).into_keys().collect();
    let mut declared: Vec<String> = BASE_KEYS.iter().map(|key| (*key).to_owned()).collect();
    declared.sort();
    assert_eq!(written, declared);
}

#[test]
fn keys_param_keys_stay_out_of_the_base_list() {
    assert!(PARAM_KEYS.iter().all(|key| !BASE_KEYS.contains(key)));
}

#[test]
fn keys_bool_and_number_keys_are_declared() {
    assert!(
        BOOL_KEYS
            .iter()
            .chain(NUMBER_KEYS.iter())
            .all(|key| BASE_KEYS.contains(key))
    );
}

#[rstest]
#[case::input_zero(r#""max_input_chars": 0"#, "max_input_chars")]
#[case::input_negative(r#""max_input_chars": -1"#, "max_input_chars")]
#[case::input_text(r#""max_input_chars": "many""#, "max_input_chars")]
#[case::concurrent_zero(
    r#""max_concurrent_decision_calls": 0"#,
    "max_concurrent_decision_calls"
)]
#[case::violation_unknown(r#""on_violation": "explode""#, "on_violation")]
#[case::violation_uppercase(r#""on_violation": "BLOCK""#, "on_violation")]
#[case::fallback_unknown(r#""unreachable_fallback": "never""#, "unreachable_fallback")]
#[case::sensitive_unknown(r#""on_sensitive_data": "drop""#, "on_sensitive_data")]
#[case::fail_on_error_text(r#""fail_on_error": "yes""#, "fail_on_error")]
#[case::scan_flag_text(r#""only_scan_new_messages": "yes""#, "only_scan_new_messages")]
#[case::handling_flag_number(
    r#""skip_unscannable_attachments": 3"#,
    "skip_unscannable_attachments"
)]
#[case::end_session_text(r#""end_session_after_n_fails": "x""#, "end_session_after_n_fails")]
fn limits_refuse_bad_values(#[case] fields: &str, #[case] field: &str) {
    let body = entry(fields);
    assert_eq!(refused_field(base(&body)), field);
    assert_eq!(refused_field(params(&body)), field);
}

#[test]
fn limits_accept_the_smallest_positive_values() {
    let parsed = base(
        r#"{"max_input_chars": 1, "max_concurrent_decision_calls": 1, "on_violation": "warn"}"#,
    )
    .unwrap();
    assert_eq!(parsed.fields.max_input_chars, NonZeroU64::new(1));
    assert_eq!(
        parsed.fields.max_concurrent_decision_calls,
        NonZeroU64::new(1)
    );
}

#[test]
fn extras_keep_unknown_keys_unchanged() {
    let parsed = base(r#"{"aws_region_name": "us-east-1", "nested": {"a": [1, "two"]}}"#).unwrap();
    let nested = GuardrailValue::Object(BTreeMap::from([(
        "a".to_owned(),
        GuardrailValue::Array(vec![GuardrailValue::SignedInteger(1), text("two")]),
    )]));
    assert_eq!(parsed.extra.0["aws_region_name"], text("us-east-1"));
    assert_eq!(parsed.extra.0["nested"], nested);
    let written = dump(&parsed);
    assert_eq!(written["aws_region_name"], text("us-east-1"));
    assert_eq!(written["nested"], nested);
}

#[test]
fn extras_keep_float_numbers() {
    let parsed = base(r#"{"top_p": 1.5, "timeout": 2.5}"#).unwrap();
    assert_eq!(parsed.extra.0["top_p"], GuardrailValue::Decimal(1.5));
    assert_eq!(parsed.fields.timeout, Some(2.5_f64));
    let decoded =
        decode_litellm_params(entry(r#""top_p": 1.5, "timeout": 0.25"#).as_bytes()).unwrap();
    assert_eq!(decoded.base.extra.0["top_p"], GuardrailValue::Decimal(1.5));
    assert_eq!(decoded.base.fields.timeout, Some(0.25_f64));
}

#[test]
fn extras_keep_integer_and_exponent_numbers() {
    let parsed = base(r#"{"big": 18446744073709551615, "negative": -5, "exponent": 1e3}"#).unwrap();
    assert_eq!(
        parsed.extra.0["big"],
        GuardrailValue::UnsignedInteger(u64::MAX)
    );
    assert_eq!(
        parsed.extra.0["negative"],
        GuardrailValue::SignedInteger(-5)
    );
    assert_eq!(parsed.extra.0["exponent"], GuardrailValue::Decimal(1000.0));
}

#[test]
fn extras_keep_the_param_keys_on_the_base_type() {
    let parsed = base(
        r#"{"guardrail": "g", "mode": "pre_call", "decision_model": "m", "checks": [{"name": "n", "instructions": "i"}]}"#,
    )
    .unwrap();
    assert!(
        PARAM_KEYS
            .iter()
            .all(|key| parsed.extra.0.contains_key(*key))
    );
}

#[test]
fn extras_drop_the_param_keys_on_litellm_params() {
    let parsed = params(
        r#"{"guardrail": "decision_model", "mode": "pre_call", "decision_model": "m", "checks": [{"name": "n", "instructions": "i"}], "api_version": "v1"}"#,
    )
    .unwrap();
    assert!(
        PARAM_KEYS
            .iter()
            .all(|key| !parsed.base.extra.0.contains_key(*key))
    );
    assert_eq!(parsed.base.extra.0["api_version"], text("v1"));
    assert_eq!(parsed.guardrail, "decision_model");
    assert_eq!(parsed.decision_model.as_deref(), Some("m"));
}

#[test]
fn lowercase_enum_text_on_both_types() {
    let body = entry(r#""unreachable_fallback": "FAIL_OPEN", "on_sensitive_data": "Block""#);
    let from_base = base(&body).unwrap();
    let from_params = params(&body).unwrap();
    assert_eq!(
        from_base.fields.unreachable_fallback,
        UnreachableFallback::FailOpen
    );
    assert_eq!(
        from_base.fields.on_sensitive_data,
        Some(OnSensitiveData::Block)
    );
    assert_eq!(
        from_params.base.fields.unreachable_fallback,
        UnreachableFallback::FailOpen
    );
    assert_eq!(
        from_params.base.fields.on_sensitive_data,
        Some(OnSensitiveData::Block)
    );
}

#[test]
fn lowercase_mode_text() {
    let parsed = params(r#"{"guardrail": "g", "mode": "PRE_Call"}"#).unwrap();
    assert_eq!(parsed.mode, LitellmParamsMode::One("pre_call".to_owned()));
}

#[test]
fn lowercase_mode_list_items() {
    let parsed = params(r#"{"guardrail": "g", "mode": ["Pre_Call", "POST_CALL"]}"#).unwrap();
    assert_eq!(
        parsed.mode,
        LitellmParamsMode::Many(vec!["pre_call".to_owned(), "post_call".to_owned()])
    );
}

#[test]
fn lowercase_tool_keys_on_litellm_params() {
    let parsed = params(&entry(
        r#""default_action": "BLOCK", "on_disallowed_action": ["Rewrite", 1]"#,
    ))
    .unwrap();
    assert_eq!(parsed.base.extra.0["default_action"], text("block"));
    assert_eq!(
        parsed.base.extra.0["on_disallowed_action"],
        GuardrailValue::Array(vec![text("rewrite"), GuardrailValue::SignedInteger(1)])
    );
}

#[test]
fn lowercase_leaves_tool_keys_alone_on_the_base_type() {
    let parsed = base(r#"{"default_action": "BLOCK", "mode": "PRE_CALL"}"#).unwrap();
    assert_eq!(parsed.extra.0["default_action"], text("BLOCK"));
    assert_eq!(parsed.extra.0["mode"], text("PRE_CALL"));
}

#[test]
fn mode_tags_keep_their_case() {
    let parsed = params(
        r#"{"guardrail": "g", "mode": {"tags": {"Team-A": ["pre_call"], "Team-B": "Post_Call"}, "default": "Pre_Call"}}"#,
    )
    .unwrap();
    let expected = LitellmParamsMode::Tagged(Mode {
        tags: BTreeMap::from([
            (
                "Team-A".to_owned(),
                ModeTarget::Many(vec!["pre_call".to_owned()]),
            ),
            ("Team-B".to_owned(), ModeTarget::One("Post_Call".to_owned())),
        ]),
        default: Some(ModeTarget::One("Pre_Call".to_owned())),
    });
    assert_eq!(parsed.mode, expected);
}

#[test]
fn mode_tags_without_default() {
    let parsed = params(r#"{"guardrail": "g", "mode": {"tags": {}}}"#).unwrap();
    assert_eq!(
        parsed.mode,
        LitellmParamsMode::Tagged(Mode {
            tags: BTreeMap::new(),
            default: None,
        })
    );
}

#[rstest]
#[case::no_tags(r#"{"default": "pre_call"}"#)]
#[case::number("5")]
#[case::list_of_numbers("[1]")]
fn mode_refuses_other_shapes(#[case] mode: &str) {
    let body = format!(r#"{{"guardrail": "g", "mode": {mode}}}"#);
    assert!(matches!(
        params(&body),
        Err(GuardrailParamsError::InvalidField { .. })
    ));
}

#[rstest]
#[case::no_mode(r#"{"guardrail": "g"}"#, "mode")]
#[case::null_mode(r#"{"guardrail": "g", "mode": null}"#, "mode")]
#[case::no_guardrail(r#"{"mode": "pre_call"}"#, "guardrail")]
#[case::null_guardrail(r#"{"guardrail": null, "mode": "pre_call"}"#, "guardrail")]
fn mode_missing_field_is_named(#[case] body: &str, #[case] missing: &str) {
    let error = params(body).unwrap_err();
    assert!(matches!(error, GuardrailParamsError::MissingField { field } if field == missing));
}

#[rstest]
#[case::route_without_model(r#""on_sensitive_data": "route""#, false)]
#[case::route_null_model(
    r#""on_sensitive_data": "route", "sensitive_data_route_to_model": null"#,
    false
)]
#[case::route_empty_model(
    r#""on_sensitive_data": "route", "sensitive_data_route_to_model": """#,
    false
)]
#[case::route_uppercase(r#""on_sensitive_data": "ROUTE""#, false)]
#[case::route_with_model(
    r#""on_sensitive_data": "route", "sensitive_data_route_to_model": "gpt-4o""#,
    true
)]
#[case::block_without_model(r#""on_sensitive_data": "block""#, true)]
#[case::model_without_route(r#""sensitive_data_route_to_model": "gpt-4o""#, true)]
fn route_rule_needs_a_model(#[case] fields: &str, #[case] accepted: bool) {
    let body = entry(fields);
    assert_eq!(base(&body).is_ok(), accepted);
    assert_eq!(params(&body).is_ok(), accepted);
}

#[test]
fn route_rule_message_matches_litellm() {
    let error = base(r#"{"on_sensitive_data": "route"}"#).unwrap_err();
    assert!(matches!(error, GuardrailParamsError::RouteModelMissing));
    assert_eq!(
        error.to_string(),
        "sensitive_data_route_to_model must be set when on_sensitive_data='route'"
    );
}

#[rstest]
#[case::block_elsewhere("custom", "block", false)]
#[case::alert_elsewhere("custom", "alert", false)]
#[case::block_on_mcp("mcp_security", "block", true)]
#[case::alert_on_mcp("mcp_security", "alert", true)]
#[case::warn_elsewhere("custom", "warn", true)]
#[case::end_session_elsewhere("custom", "end_session", true)]
fn violation_rule_needs_mcp_security(
    #[case] guardrail: &str,
    #[case] violation: &str,
    #[case] accepted: bool,
) {
    let body = format!(
        r#"{{"guardrail": "{guardrail}", "mode": "pre_call", "on_violation": "{violation}"}}"#
    );
    let result = params(&body);
    assert_eq!(result.is_ok(), accepted);
    if let Err(error) = result {
        assert_eq!(
            error.to_string(),
            format!("on_violation='{violation}' is only supported by guardrail='mcp_security'")
        );
    }
}

#[test]
fn violation_rule_does_not_apply_to_the_base_type() {
    let parsed = base(r#"{"on_violation": "block"}"#).unwrap();
    assert_eq!(
        parsed
            .fields
            .on_violation
            .map(|value| value.as_ref().to_owned()),
        Some("block".to_owned())
    );
}

#[rstest]
#[case::list_elsewhere("custom", r#"[{"name": "n", "instructions": "i"}]"#)]
#[case::empty_list_elsewhere("custom", "[]")]
fn checks_rule_needs_decision_model(#[case] guardrail: &str, #[case] checks: &str) {
    let body = format!(r#"{{"guardrail": "{guardrail}", "mode": "pre_call", "checks": {checks}}}"#);
    let error = params(&body).unwrap_err();
    assert!(matches!(
        error,
        GuardrailParamsError::ChecksNeedDecisionModel { .. }
    ));
    assert_eq!(
        error.to_string(),
        "checks as a list is only supported by guardrail='decision_model', got guardrail='custom'"
    );
}

#[test]
fn checks_rule_accepts_a_list_for_decision_model() {
    let parsed = params(
        r#"{"guardrail": "decision_model", "mode": "pre_call", "checks": [{"name": "n", "instructions": "i"}]}"#,
    )
    .unwrap();
    match parsed.checks {
        Some(GuardrailChecks::Decision(checks)) => assert_eq!(checks.len(), 1),
        other => panic!("expected the list form, got {other:?}"),
    }
}

#[test]
fn checks_rule_accepts_bedrock_checks_elsewhere() {
    let parsed = params(
        r#"{"guardrail": "custom", "mode": "pre_call", "checks": {"contentFilter": {"categories": [{"category": "HATE"}]}}}"#,
    )
    .unwrap();
    assert!(matches!(parsed.checks, Some(GuardrailChecks::Bedrock(_))));
}

#[test]
fn checks_rule_refuses_an_empty_mapping() {
    let result = params(r#"{"guardrail": "custom", "mode": "pre_call", "checks": {}}"#);
    assert!(matches!(
        result,
        Err(GuardrailParamsError::InvalidField { .. })
    ));
}

#[rstest]
#[case::empty_name(r#"{"name": "", "instructions": "i"}"#)]
#[case::empty_instructions(r#"{"name": "n", "instructions": ""}"#)]
#[case::blank_instructions(r#"{"name": "n", "instructions": " \n\t "}"#)]
#[case::threshold_above_one(r#"{"name": "n", "instructions": "i", "threshold": 1.01}"#)]
#[case::threshold_below_zero(r#"{"name": "n", "instructions": "i", "threshold": -0.01}"#)]
#[case::unknown_action(r#"{"name": "n", "instructions": "i", "action": "explode"}"#)]
#[case::missing_name(r#"{"instructions": "i"}"#)]
fn decision_check_refuses_bad_input(#[case] body: &str) {
    serde_json::from_str::<DecisionModelCheck>(body).unwrap_err();
}

#[rstest]
#[case::lowest("0.0")]
#[case::highest("1.0")]
fn decision_check_accepts_threshold_edges(#[case] threshold: &str) {
    let body = format!(r#"{{"name": "n", "instructions": "i", "threshold": {threshold}}}"#);
    serde_json::from_str::<DecisionModelCheck>(&body).unwrap();
}

#[test]
fn decision_check_defaults_and_dump() {
    let check: DecisionModelCheck =
        serde_json::from_str(r#"{"name": "n", "instructions": "i"}"#).unwrap();
    assert_eq!(check.threshold.to_bits(), 0.7_f64.to_bits());
    assert_eq!(check.action, DecisionCheckAction::Block);
    let written = serde_json::to_string(&check).unwrap();
    assert!(written.contains(r#""threshold":0.7"#));
    assert!(written.contains(r#""action":"block""#));
    let logged: DecisionModelCheck =
        serde_json::from_str(r#"{"name": "n", "instructions": "i", "action": "log"}"#).unwrap();
    assert_eq!(logged.action, DecisionCheckAction::Log);
}

#[rstest]
#[case::number("30", Some(30.0_f64))]
#[case::float("1.5", Some(1.5_f64))]
#[case::numeric_text(r#""30""#, Some(30.0_f64))]
#[case::float_text(r#""1.5""#, Some(1.5_f64))]
#[case::empty_text(r#""""#, None)]
fn timeout_is_coerced_on_litellm_params(#[case] timeout: &str, #[case] expected: Option<f64>) {
    let parsed = params(&entry(&format!(r#""timeout": {timeout}"#))).unwrap();
    assert_eq!(parsed.base.fields.timeout, expected);
}

#[rstest]
#[case::letters("abc")]
#[case::blank("   ")]
#[case::not_a_number("nan")]
#[case::infinity("inf")]
#[case::negative_infinity("-inf")]
#[case::underscores("1_000")]
fn timeout_refuses_other_text(#[case] timeout: &str) {
    let error = params(&entry(&format!(r#""timeout": "{timeout}""#))).unwrap_err();
    assert!(matches!(error, GuardrailParamsError::InvalidTimeout));
    assert_eq!(error.to_string(), "timeout must be numeric");
}

#[test]
fn timeout_text_is_refused_on_the_base_type() {
    assert_eq!(refused_field(base(r#"{"timeout": "30"}"#)), "timeout");
    assert_eq!(
        base(r#"{"timeout": 30}"#).unwrap().fields.timeout,
        Some(30.0_f64)
    );
}

#[rstest]
#[case::absent("", Some(false))]
#[case::null(r#", "default_on": null"#, Some(false))]
#[case::true_kept(r#", "default_on": true"#, Some(true))]
#[case::false_kept(r#", "default_on": false"#, Some(false))]
fn default_on_is_false_on_litellm_params(#[case] fields: &str, #[case] expected: Option<bool>) {
    let parsed = params(&format!("{{{ENTRY}{fields}}}")).unwrap();
    assert_eq!(parsed.base.fields.default_on, expected);
}

#[test]
fn default_on_stays_unset_on_the_base_type() {
    let parsed = base("{}").unwrap();
    assert_eq!(parsed.fields.default_on, None);
    assert_eq!(dump(&parsed)["default_on"], GuardrailValue::Null);
    let filled = params(&entry("\"api_version\": \"v1\"")).unwrap();
    assert_eq!(dump(&filled)["default_on"], GuardrailValue::Boolean(false));
}

#[test]
fn sent_fields_return_only_sent_keys() {
    let sent = base(r#"{"guard_name": "g", "max_input_chars": 5, "custom_key": 1.5}"#)
        .unwrap()
        .sent_fields()
        .unwrap();
    let expected = BTreeMap::from([
        ("guard_name".to_owned(), text("g")),
        (
            "max_input_chars".to_owned(),
            GuardrailValue::SignedInteger(5),
        ),
        ("custom_key".to_owned(), GuardrailValue::Decimal(1.5)),
    ]);
    assert_eq!(sent, expected);
}

#[test]
fn sent_fields_keep_an_explicit_null() {
    let sent = base(r#"{"api_base": null, "fail_on_error": null, "extra_null": null}"#)
        .unwrap()
        .sent_fields()
        .unwrap();
    let expected = BTreeMap::from([
        ("api_base".to_owned(), GuardrailValue::Null),
        ("fail_on_error".to_owned(), GuardrailValue::Null),
        ("extra_null".to_owned(), GuardrailValue::Null),
    ]);
    assert_eq!(sent, expected);
}

#[test]
fn sent_fields_return_lowercased_values() {
    let sent = base(r#"{"unreachable_fallback": "FAIL_OPEN"}"#)
        .unwrap()
        .sent_fields()
        .unwrap();
    assert_eq!(sent["unreachable_fallback"], text("fail_open"));
}

#[test]
fn sent_fields_are_empty_for_an_empty_body() {
    assert!(base("{}").unwrap().sent_fields().unwrap().is_empty());
}

const SECRET_BODY: &str = r#"{"guardrail": "custom", "mode": "pre_call", "api_key": "sk-leak-1", "credentials": "cred-leak-2", "client_secret": "extra-leak-3"}"#;

#[rstest]
#[case::api_key_value("sk-leak-1")]
#[case::credentials_value("cred-leak-2")]
#[case::extra_value("extra-leak-3")]
#[case::info_value("info-leak-4")]
#[case::api_key_name("api_key")]
#[case::credentials_name("credentials")]
fn secret_debug_text_hides_values(#[case] needle: &str) {
    let parsed = params(SECRET_BODY).unwrap();
    let guardrail = Guardrail {
        guardrail_id: None,
        guardrail_name: "safety".to_owned(),
        litellm_params: parsed.clone(),
        guardrail_info: Some(opaque(r#"{"token": "info-leak-4"}"#)),
        policy_template: None,
        created_at: None,
        updated_at: None,
        team_id: None,
    };
    let settings = Settings {
        guardrails: vec![guardrail.clone()],
        rejected: Vec::new(),
    };
    let shown = format!(
        "{:?} {:?} {guardrail:?} {settings:?}",
        base(SECRET_BODY).unwrap(),
        parsed
    );
    assert!(!shown.contains(needle), "{needle} leaked");
}

#[test]
fn secret_dump_keeps_the_key_text() {
    let written = dump(&base(SECRET_BODY).unwrap());
    assert_eq!(written["api_key"], text("sk-leak-1"));
    assert_eq!(written["credentials"], text("cred-leak-2"));
    assert_eq!(written["client_secret"], text("extra-leak-3"));
}

#[rstest]
#[case::input_chars(r#""max_input_chars": "leak-5""#)]
#[case::violation(r#""on_violation": "leak-6""#)]
#[case::timeout(r#""timeout": "leak-7""#)]
#[case::fallback(r#""unreachable_fallback": "leak-8""#)]
#[case::route(r#""on_sensitive_data": "leak-9""#)]
fn secret_error_text_hides_the_bad_value(#[case] fields: &str) {
    let body = entry(fields);
    let from_params = params(&body).unwrap_err();
    let from_decoder = decode_litellm_params(body.as_bytes()).unwrap_err();
    let shown = format!("{from_params} {from_params:?} {from_decoder} {from_decoder:?}");
    assert!(!shown.contains("leak-"), "{shown}");
}

#[test]
fn values_expecting_text_names_the_parameter_value() {
    let bytes = BytesDeserializer::<ValueError>::new(b"x");
    let error = ExactValue::deserialize(bytes).err().unwrap();
    assert!(error.to_string().contains("a guardrail parameter value"));
}

#[test]
fn values_read_both_null_forms_as_null() {
    let unit = ValueVisitor.visit_unit::<ValueError>().unwrap();
    let none = ValueVisitor.visit_none::<ValueError>().unwrap();
    assert_eq!(unit.0, GuardrailValue::Null);
    assert_eq!(none.0, GuardrailValue::Null);
}

#[test]
fn debug_text_lists_the_key_names_only() {
    let shown = format!(
        "{:?}",
        opaque(r#"{"token": "leak-1", "api_base": "leak-2"}"#)
    );
    assert_eq!(shown, r#"["api_base", "token"]"#);
    let held = format!("{:?}", base(r#"{"custom": "leak-3"}"#).unwrap());
    assert!(held.starts_with("BaseLitellmParams"), "{held}");
    assert!(held.contains("custom"), "{held}");
    assert!(!held.contains("leak-"), "{held}");
}

mod env;
mod records;
mod section;
