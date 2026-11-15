#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::{LitellmParamsMode, Mode, ModeTarget};
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_records::invocation::{
    GuardrailHookName, GuardrailInvocation,
};
use betterlite_guardrails::guardrail_records::record::GuardrailStatus;
use rstest::rstest;

use super::settings::SettingsJson;
use super::{
    HOOK_NAMES, event_type_from_name, exact_data, exact_value, find, is_integer_literal, read,
    settings_from_text, suppressed_from_text, text_context_from_text,
};
use crate::guardrail_call::context::{RecordStep, log_target_from_text, record_call, record_parts};
use crate::guardrail_call::convert::record_input::record_input_from_text;
use crate::guardrail_call::error::CallError;
use crate::guardrail_call::native::NativeGuardrail;

fn value(text: &str) -> Result<GuardrailValue, CallError> {
    read("guardrail response", text).and_then(exact_value)
}

fn body(text: &str) -> BTreeMap<String, GuardrailValue> {
    let raw: BTreeMap<String, GuardrailValue> = read("request data", text).unwrap();
    exact_data(raw).unwrap()
}

const FULL_SETTINGS: &str = r#"{
    "guardrail_name": "pii",
    "supported_event_hooks": ["pre_call", "post_call"],
    "event_hook": ["pre_call", "post_call"],
    "default_on": true,
    "mask_request_content": true,
    "mask_response_content": true,
    "violation_message_template": "blocked: {default_message}",
    "end_session_after_n_fails": 3,
    "on_violation": "end_session",
    "realtime_violation_message": "flagged",
    "on_sensitive_data": "route",
    "sensitive_data_route_to_model": "gpt-4o",
    "sticky_session_routing": false,
    "run_in_parallel": true,
    "scan_raw_request": true,
    "only_scan_new_messages": true,
    "timeout": 12.5,
    "switches": {
        "use_native_during_call_hook": true,
        "use_native_lifecycle_hooks": true,
        "records_own_information": true,
        "uses_apply_guardrail_interface": true,
        "deny_text_on_error": true
    }
}"#;

fn call_error(text: &str) -> CallError {
    settings_from_text(text).unwrap_err()
}

#[test]
fn settings_default_from_a_name() {
    let settings = settings_from_text(r#"{"guardrail_name": "pii"}"#).unwrap();
    assert_eq!(settings.guardrail_name.as_deref(), Some("pii"));
    assert_eq!(settings.supported_event_hooks, None);
    assert_eq!(settings.event_hook, None);
    assert!(!settings.default_on);
    assert!(!settings.mask_request_content);
    assert!(!settings.mask_response_content);
    assert_eq!(settings.violation_message_template, None);
    assert_eq!(settings.timeout, None);
    assert!(settings.sticky_session_routing);
    assert!(!settings.run_in_parallel);
    assert!(!settings.only_scan_new_messages);
    assert!(!settings.switches.deny_text_on_error);
}

#[test]
fn settings_read_the_name_and_hooks() {
    let settings = settings_from_text(FULL_SETTINGS).unwrap();
    assert_eq!(settings.guardrail_name.as_deref(), Some("pii"));
    assert_eq!(
        settings.supported_event_hooks,
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall
        ])
    );
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::Many(vec![
            "pre_call".to_owned(),
            "post_call".to_owned()
        ]))
    );
}

#[test]
fn settings_read_the_policy_flags() {
    let settings = settings_from_text(FULL_SETTINGS).unwrap();
    assert!(settings.default_on);
    assert!(settings.mask_request_content);
    assert!(settings.mask_response_content);
    assert_eq!(
        settings.violation_message_template.as_deref(),
        Some("blocked: {default_message}")
    );
    assert_eq!(settings.end_session_after_n_fails, Some(3));
    assert_eq!(settings.on_violation.as_deref(), Some("end_session"));
    assert_eq!(
        settings.realtime_violation_message.as_deref(),
        Some("flagged")
    );
    assert_eq!(settings.on_sensitive_data.as_deref(), Some("route"));
    assert_eq!(
        settings.sensitive_data_route_to_model.as_deref(),
        Some("gpt-4o")
    );
    assert!(!settings.sticky_session_routing);
    assert!(settings.run_in_parallel);
    assert!(settings.scan_raw_request);
    assert!(settings.only_scan_new_messages);
    assert_eq!(settings.timeout, Some(12.5_f64));
}

#[test]
fn settings_read_the_switches() {
    let settings = settings_from_text(FULL_SETTINGS).unwrap();
    assert!(settings.switches.use_native_during_call_hook);
    assert!(settings.switches.use_native_lifecycle_hooks);
    assert!(settings.switches.records_own_information);
    assert!(settings.switches.uses_apply_guardrail_interface);
    assert!(settings.switches.deny_text_on_error);
}

#[test]
fn settings_round_trip_through_the_json_shape() {
    let parsed: SettingsJson = serde_json::from_str(FULL_SETTINGS).unwrap();
    let settings = parsed.clone().into_settings();
    assert_eq!(SettingsJson::from(&settings), parsed);
}

#[test]
fn settings_read_a_tagged_event_hook() {
    let text = r#"{"event_hook": {"tags": {"health": "pre_call", "chat": ["post_call", "during_call"]}, "default": "logging_only"}}"#;
    let settings = settings_from_text(text).unwrap();
    let expected = Mode {
        tags: BTreeMap::from([
            (
                "chat".to_owned(),
                ModeTarget::Many(vec!["post_call".to_owned(), "during_call".to_owned()]),
            ),
            ("health".to_owned(), ModeTarget::One("pre_call".to_owned())),
        ]),
        default: Some(ModeTarget::One("logging_only".to_owned())),
    };
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::Tagged(expected))
    );
}

#[test]
fn settings_reject_an_unknown_key() {
    let error = call_error(r#"{"guardrail_name": "pii", "sticky_sessions": true}"#);
    assert_eq!(error.kind(), "InvalidJson");
    assert!(error.to_string().contains("sticky_sessions"), "{error}");
}

#[rstest]
#[case::number("3")]
#[case::list_of_numbers("[3]")]
#[case::list_with_an_object("[{\"a\": 1}]")]
#[case::object_without_tags("{\"default\": \"pre_call\"}")]
#[case::tags_not_an_object("{\"tags\": 3}")]
fn settings_reject_a_bad_event_hook(#[case] event_hook: &str) {
    let text = format!(r#"{{"event_hook": {event_hook}}}"#);
    let error = call_error(&text);
    assert_eq!(error.kind(), "InvalidJson");
}

#[test]
fn settings_reject_an_unsupported_hook() {
    let text = r#"{"guardrail_name": "pii", "supported_event_hooks": ["pre_call"], "event_hook": "post_call"}"#;
    let settings = settings_from_text(text).unwrap();
    let refusal = GuardrailBase::new(settings, true).unwrap_err();
    let error = CallError::from(refusal);
    assert_eq!(error.kind(), "UnsupportedEventHook");
    assert_eq!(
        error.to_string(),
        "Event hook post_call is not in the supported event hooks [pre_call]"
    );
    assert!(NativeGuardrail::new(text, true).is_err());
    NativeGuardrail::new(text, false).unwrap();
}

#[test]
fn settings_reject_text_that_is_not_json() {
    let error = call_error("{");
    assert_eq!(error.kind(), "InvalidJson");
    assert!(NativeGuardrail::new("not json", true).is_err());
}

#[test]
fn settings_update_params_follow_the_given_params() {
    let text = r#"{"guardrail_name": "pii", "event_hook": "pre_call", "default_on": false}"#;
    let settings = settings_from_text(text).unwrap();
    let mut base = GuardrailBase::new(settings, true).unwrap();
    let params = read::<LitellmParams>(
            "guardrail params",
        r#"{"guardrail": "pii", "mode": "post_call", "default_on": true, "mask_request_content": true,
            "violation_message_template": "stop", "timeout": 4.5, "on_violation": "warn"}"#,
    )
    .unwrap();
    base.update_params(&params);
    let updated = SettingsJson::from(&base.settings);
    assert!(updated.default_on);
    assert!(updated.mask_request_content);
    assert_eq!(updated.violation_message_template.as_deref(), Some("stop"));
    assert_eq!(updated.on_violation.as_deref(), Some("warn"));
    assert_eq!(updated.timeout, Some(4.5_f64));
    assert_eq!(
        updated.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
    assert_eq!(
        base.mode,
        Some(LitellmParamsMode::One("post_call".to_owned()))
    );
}

#[test]
fn value_keeps_floats_outside_the_serde_number_marker() {
    assert_eq!(value("1.5").unwrap(), GuardrailValue::Decimal(1.5));
    assert_eq!(
        value(r#"{"a": [1.5, {"b": 2}]}"#).unwrap(),
        GuardrailValue::Object(BTreeMap::from([(
            "a".to_owned(),
            GuardrailValue::Array(vec![
                GuardrailValue::Decimal(1.5),
                GuardrailValue::Object(BTreeMap::from([(
                    "b".to_owned(),
                    GuardrailValue::SignedInteger(2)
                )]))
            ])
        )]))
    );
}

#[test]
fn value_round_trips_through_text() {
    let text = r#"{"text":"a","flag":true,"none":null,"list":["x",2]}"#;
    let parsed = value(text).unwrap();
    let again = value(&serde_json::to_string(&parsed).unwrap()).unwrap();
    assert_eq!(again, parsed);
    assert_eq!(
        serde_json::to_string(&GuardrailValue::Text("hi".to_owned())).unwrap(),
        "\"hi\""
    );
}

#[test]
fn data_reads_null_and_a_list_wrongly() {
    let absent: Option<BTreeMap<String, GuardrailValue>> = read("request data", "null").unwrap();
    assert_eq!(absent, None);
    let present: Option<BTreeMap<String, GuardrailValue>> =
        read("request data", r#"{"messages": []}"#).unwrap();
    assert_eq!(
        present.map(exact_data).transpose().unwrap().unwrap().len(),
        1
    );
    let error: CallError =
        read::<Option<BTreeMap<String, GuardrailValue>>>("request data", "[]").unwrap_err();
    assert_eq!(error.kind(), "InvalidJson");
    assert_eq!(
        serde_json::to_string(&BTreeMap::from([(
            "score".to_owned(),
            GuardrailValue::Decimal(0.5)
        )]))
        .unwrap(),
        r#"{"score":0.5}"#
    );
    let parsed: BTreeMap<String, GuardrailValue> =
        read("request data", r#"{"score": 0.5}"#).unwrap();
    assert_eq!(
        exact_data(parsed).unwrap()["score"],
        GuardrailValue::Decimal(0.5)
    );
}

#[test]
fn value_absent_reads_none() {
    let absent: Option<GuardrailValue> = read("guardrail response", "null").unwrap();
    assert_eq!(absent, None);
    let present: Option<GuardrailValue> = read("guardrail response", "\"deny\"").unwrap();
    assert_eq!(
        present.map(exact_value).transpose().unwrap(),
        Some(GuardrailValue::Text("deny".to_owned()))
    );
}

#[rstest]
#[case("pre_call", GuardrailEventHooks::PreCall)]
#[case("post_call", GuardrailEventHooks::PostCall)]
#[case("during_call", GuardrailEventHooks::DuringCall)]
#[case("logging_only", GuardrailEventHooks::LoggingOnly)]
#[case("pre_mcp_call", GuardrailEventHooks::PreMcpCall)]
#[case("during_mcp_call", GuardrailEventHooks::DuringMcpCall)]
#[case("post_mcp_call", GuardrailEventHooks::PostMcpCall)]
#[case(
    "realtime_input_transcription",
    GuardrailEventHooks::RealtimeInputTranscription
)]
fn event_types_read_every_hook(#[case] name: &str, #[case] hook: GuardrailEventHooks) {
    assert_eq!(event_type_from_name(name).unwrap(), hook);
}

#[test]
fn event_types_reject_an_unknown_name() {
    let error = event_type_from_name("before_call").unwrap_err();
    assert_eq!(error.kind(), "InvalidEventType");
    assert!(error.to_string().contains("before_call"), "{error}");
}

#[rstest]
#[case("async_pre_call_hook", GuardrailHookName::PreCall)]
#[case("async_moderation_hook", GuardrailHookName::Moderation)]
#[case("async_post_call_success_hook", GuardrailHookName::PostCallSuccess)]
#[case("async_post_call_streaming_hook", GuardrailHookName::PostCallStreaming)]
#[case(
    "async_post_call_streaming_iterator_hook",
    GuardrailHookName::StreamingIterator
)]
#[case("apply_guardrail", GuardrailHookName::ApplyGuardrail)]
fn hook_names_read_every_wrapper(#[case] name: &str, #[case] hook: GuardrailHookName) {
    assert_eq!(find(&HOOK_NAMES, name), Some(hook));
}

#[test]
fn hook_names_leave_an_unknown_wrapper_unmatched() {
    assert_eq!(find(&HOOK_NAMES, "async_custom_hook"), None);
}

#[test]
fn hooks_with_an_unknown_name_use_the_configured_mode() {
    let text = r#"{"messages": []}"#;
    let data = body(text);
    let hook = GuardrailHookName::StreamingIterator;
    let mut invocation =
        Some(GuardrailInvocation::begin(hook, &data, None, SystemTime::now()).unwrap());
    let context = r#"{
        "settings": {"guardrail_name": "pii", "event_hook": "pre_call"},
        "log_target": null
    }"#;
    let (recorded, _target) =
        record_call(&mut invocation, RecordStep::Finish(None), text, context).unwrap();
    assert!(
        recorded.contains("\"guardrail_mode\":\"pre_call\""),
        "{recorded}"
    );
}

#[test]
fn the_number_marker_reaches_rust_as_a_number() {
    assert_eq!(
        value(r#"{"$serde_json::private::Number": "12"}"#).unwrap(),
        GuardrailValue::SignedInteger(12)
    );
}

#[test]
fn data_rejects_an_integer_past_u64() {
    let error = value(r#"{"trace": 123456789012345678901234567890}"#).unwrap_err();
    assert_eq!(error.kind(), "InvalidJson");
    assert!(
        error.to_string().contains("123456789012345678901234567890"),
        "{error}"
    );
}

#[test]
fn integer_literals_keep_their_sign_and_digits() {
    assert!(is_integer_literal("42"));
    assert!(is_integer_literal("-7"));
    assert!(!is_integer_literal("1.5"));
    assert!(!is_integer_literal("-"));
    assert!(!is_integer_literal(""));
}

#[test]
fn encode_errors_keep_the_json_kind() {
    let source = serde_json::from_str::<GuardrailValue>("not json").unwrap_err();
    let error = CallError::Encode { source };
    assert_eq!(error.kind(), "InvalidJson");
    assert!(error.to_string().contains("written as JSON"), "{error}");
}

#[test]
fn context_reads_text_values_and_names_a_bad_one() {
    let entries = text_context_from_text(r#"{"reason": "pii", "count": "2"}"#).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.contains(&("reason".to_owned(), "pii".to_owned())));
    let error = text_context_from_text(r#"{"reason": 2}"#).unwrap_err();
    assert_eq!(error.kind(), "ContextValueNotText");
    assert!(error.to_string().contains("reason"), "{error}");
    assert_eq!(
        text_context_from_text("[]").unwrap_err().kind(),
        "InvalidJson"
    );
}

#[test]
fn suppressed_reads_names_once() {
    let names = suppressed_from_text(r#"["pii", "pii", "secrets"]"#).unwrap();
    assert_eq!(
        names,
        BTreeSet::from(["pii".to_owned(), "secrets".to_owned()])
    );
    assert_eq!(
        suppressed_from_text("null").unwrap_err().kind(),
        "InvalidJson"
    );
}

#[test]
fn params_read_a_guardrail_mode() {
    let params = read::<LitellmParams>(
        "guardrail params",
        r#"{"guardrail": "pii", "mode": ["pre_call"]}"#,
    )
    .unwrap();
    assert_eq!(
        params.mode,
        LitellmParamsMode::Many(vec!["pre_call".to_owned()])
    );
    assert_eq!(params.guardrail, "pii");
    let error = read::<LitellmParams>("guardrail params", r#"{"mode": "pre_call"}"#).unwrap_err();
    assert_eq!(error.kind(), "InvalidJson");
}

const RECORD_INPUT: &str = r#"{
    "guardrail_response": {"verdict": "x", "score": 1.5},
    "guardrail_status": "guardrail_intervened",
    "start_time": 10.0,
    "end_time": 12.0,
    "duration": 2.0,
    "masked_entity_count": {"EMAIL": 2},
    "guardrail_provider": "pii",
    "event_type": "pre_call",
    "tracing_detail": {
        "guardrail_id": "g-1",
        "policy_template": "t",
        "detection_method": "regex",
        "confidence_score": 0.9,
        "classification": {"kind": "pii"},
        "match_details": "REDACTED",
        "patterns_checked": 4,
        "alert_recipients": ["a@example.com"],
        "risk_score": 1.5,
        "violation_categories": ["pii"],
        "guardrail_action": "block",
        "guardrail_usage": {"units": 2},
        "guardrail_cost": 1.50,
        "guardrail_cost_by_unit": {"units": 0.5},
        "guardrail_cost_in_spend": true
    }
}"#;

#[test]
fn record_input_reads_the_response_and_status() {
    let input = record_input_from_text(RECORD_INPUT).unwrap();
    assert_eq!(input.guardrail_status, GuardrailStatus::GuardrailIntervened);
    assert_eq!(input.start_time, Some(10.0_f64));
    assert_eq!(
        input.masked_entity_count,
        Some(BTreeMap::from([("EMAIL".to_owned(), 2_i64)]))
    );
    assert_eq!(input.guardrail_provider.as_deref(), Some("pii"));
    assert_eq!(input.event_type, Some(GuardrailEventHooks::PreCall));
    assert_eq!(
        input.guardrail_response,
        GuardrailValue::Object(BTreeMap::from([
            ("score".to_owned(), GuardrailValue::Decimal(1.5)),
            ("verdict".to_owned(), GuardrailValue::Text("x".to_owned())),
        ]))
    );
}

#[test]
fn record_input_reads_the_tracing_detail() {
    let input = record_input_from_text(RECORD_INPUT).unwrap();
    let detail = input.tracing_detail.unwrap();
    assert_eq!(detail.guardrail_id.as_deref(), Some("g-1"));
    assert_eq!(detail.policy_template.as_deref(), Some("t"));
    assert_eq!(detail.detection_method.as_deref(), Some("regex"));
    assert_eq!(detail.confidence_score, Some(0.9_f64));
    assert_eq!(detail.guardrail_action.as_deref(), Some("block"));
    assert_eq!(detail.patterns_checked, Some(4));
    assert_eq!(detail.risk_score, Some(1.5_f64));
    assert_eq!(
        detail.guardrail_usage,
        Some(BTreeMap::from([("units".to_owned(), 2_i64)]))
    );
    assert_eq!(
        detail.match_details,
        Some(GuardrailValue::Text("REDACTED".to_owned()))
    );
    assert_eq!(
        detail.guardrail_cost,
        Some(rust_decimal::Decimal::new(150, 2))
    );
    assert_eq!(
        detail.guardrail_cost_by_unit,
        Some(BTreeMap::from([(
            "units".to_owned(),
            Some(rust_decimal::Decimal::new(5, 1))
        )]))
    );
    assert_eq!(detail.guardrail_cost_in_spend, Some(true));
}

#[test]
fn record_input_requires_a_response() {
    let error = record_input_from_text(r#"{"guardrail_status": "success"}"#).unwrap_err();
    assert_eq!(error.kind(), "InvalidJson");
    assert!(error.to_string().contains("guardrail_response"), "{error}");
}

#[test]
fn inputs_read_texts_and_accept_null() {
    let absent: Option<GuardrailApiInputs> = read("guardrail inputs", "null").unwrap();
    assert_eq!(absent, None);
    let present: Option<GuardrailApiInputs> =
        read("guardrail inputs", r#"{"texts": ["a"], "model": "gpt"}"#).unwrap();
    let inputs = present.unwrap();
    assert_eq!(inputs.texts, Some(vec!["a".to_owned()]));
    assert_eq!(inputs.model.as_deref(), Some("gpt"));
    let error: CallError = read::<Option<GuardrailApiInputs>>("guardrail inputs", "1").unwrap_err();
    assert_eq!(error.kind(), "InvalidJson");
}

#[test]
fn record_parts_build_a_base_and_a_log_target() {
    let text = r#"{
        "settings": {"guardrail_name": "pii", "event_hook": "pre_call"},
        "log_target": {"litellm_params": {"metadata": {"k": 1}}, "model_call_params": null}
    }"#;
    let mut parts = record_parts(text).unwrap();
    let mut body = BTreeMap::new();
    let record_context = parts.context(&mut body);
    assert_eq!(
        record_context.base.settings.guardrail_name.as_deref(),
        Some("pii")
    );
    assert_eq!(
        record_context.base.settings.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
    let rendered = parts.log_target_text().unwrap();
    assert!(
        rendered.contains("\"model_call_params\":null"),
        "{rendered}"
    );
}

#[test]
fn log_targets_reject_an_integer_past_u64() {
    let target =
        r#"{"litellm_params": {"id": 123456789012345678901234567890}, "model_call_params": null}"#;
    let target_error = log_target_from_text(target).unwrap_err();
    assert_eq!(target_error.kind(), "InvalidJson");
    let context = format!(r#"{{"settings": {{}}, "log_target": {target}}}"#);
    let parts_error = record_parts(&context).err().unwrap();
    assert_eq!(parts_error.kind(), "InvalidJson");
}

#[test]
fn record_parts_reject_an_unknown_context_key() {
    let error = record_parts(r#"{"settings": {}, "log": {}}"#)
        .err()
        .unwrap();
    assert_eq!(error.kind(), "InvalidJson");
    assert!(error.to_string().contains("log"), "{error}");
    assert!(record_parts("[]").is_err());
}
