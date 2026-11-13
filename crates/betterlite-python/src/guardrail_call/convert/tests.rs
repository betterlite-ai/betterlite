#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::{LitellmParamsMode, Mode, ModeTarget};
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_records::invocation::GuardrailHookName;
use betterlite_guardrails::guardrail_records::record::GuardrailStatus;
use rstest::rstest;

use super::settings::SettingsJson;
use super::{
    data_from_text, data_to_text, event_type_from_name, hook_from_name, inputs_from_text,
    optional_data_from_text, optional_value_from_text, params_from_text, settings_from_text,
    suppressed_from_text, text_context_from_text, value_from_text, value_to_text,
};
use crate::guardrail_call::context::record_parts;
use crate::guardrail_call::convert::record_input::record_input_from_text;
use crate::guardrail_call::error::GuardrailCallError;
use crate::guardrail_call::native::NativeGuardrail;

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

fn call_error(text: &str) -> GuardrailCallError {
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
fn settings_read_every_constructor_argument() {
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
    assert_eq!(settings.timeout, Some(12.5));
    assert!(settings.switches.use_native_during_call_hook);
    assert!(settings.switches.use_native_lifecycle_hooks);
    assert!(settings.switches.records_own_information);
    assert!(settings.switches.uses_apply_guardrail_interface);
    assert!(settings.switches.deny_text_on_error);
}

#[test]
fn settings_round_trip_through_the_json_shape() {
    let parsed: SettingsJson = serde_json::from_str(FULL_SETTINGS).unwrap();
    let settings = parsed.clone().into_settings().unwrap();
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
#[case::unknown_mode_key("{\"tags\": {}, \"mode\": \"pre_call\"}")]
fn settings_reject_a_bad_event_hook(#[case] event_hook: &str) {
    let text = format!(r#"{{"event_hook": {event_hook}}}"#);
    let error = call_error(&text);
    assert_eq!(error.kind(), "InvalidEventHook");
}

#[test]
fn settings_reject_an_unsupported_hook() {
    let text = r#"{"guardrail_name": "pii", "supported_event_hooks": ["pre_call"], "event_hook": "post_call"}"#;
    let settings = settings_from_text(text).unwrap();
    let error = GuardrailBase::new(settings, true).unwrap_err();
    let error = GuardrailCallError::from(error);
    assert_eq!(error.kind(), "UnsupportedEventHook");
    assert_eq!(
        error.to_string(),
        "Event hook post_call is not in the supported event hooks [pre_call]"
    );
    assert!(NativeGuardrail::new(text, true).is_err());
    assert!(NativeGuardrail::new(text, false).is_ok());
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
    let params = params_from_text(
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
    assert_eq!(updated.timeout, Some(4.5));
    assert_eq!(
        updated.event_hook,
        Some(GuardrailValue::Text("pre_call".to_owned()))
    );
    assert_eq!(
        base.mode,
        Some(LitellmParamsMode::One("post_call".to_owned()))
    );
}

#[test]
fn value_keeps_floats_outside_the_serde_number_marker() {
    assert_eq!(
        value_from_text("1.5").unwrap(),
        GuardrailValue::Decimal(1.5)
    );
    assert_eq!(
        value_from_text(r#"{"a": [1.5, {"b": 2}]}"#).unwrap(),
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
    let value = value_from_text(text).unwrap();
    let again = value_from_text(&value_to_text(&value).unwrap()).unwrap();
    assert_eq!(again, value);
    assert_eq!(
        value_to_text(&GuardrailValue::Text("hi".to_owned())).unwrap(),
        "\"hi\""
    );
}

#[test]
fn data_reads_absent_null_and_a_list_wrongly() {
    assert_eq!(optional_data_from_text(None).unwrap(), None);
    assert_eq!(optional_data_from_text(Some("null")).unwrap(), None);
    let body = optional_data_from_text(Some(r#"{"messages": []}"#))
        .unwrap()
        .unwrap();
    assert_eq!(body.len(), 1);
    let error = optional_data_from_text(Some("[]")).unwrap_err();
    assert_eq!(error.kind(), "InvalidJson");
    assert_eq!(
        data_to_text(&BTreeMap::from([(
            "score".to_owned(),
            GuardrailValue::Decimal(0.5)
        )]))
        .unwrap(),
        r#"{"score":0.5}"#
    );
    assert_eq!(
        data_from_text(r#"{"score": 0.5}"#).unwrap()["score"],
        GuardrailValue::Decimal(0.5)
    );
}

#[test]
fn value_absent_reads_none() {
    assert_eq!(optional_value_from_text(None).unwrap(), None);
    assert_eq!(optional_value_from_text(Some("null")).unwrap(), None);
    assert_eq!(
        optional_value_from_text(Some("\"deny\"")).unwrap(),
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
#[case("apply_guardrail", GuardrailHookName::ApplyGuardrail)]
#[case("streaming_iterator", GuardrailHookName::StreamingIterator)]
fn hook_names_read_every_wrapper(#[case] name: &str, #[case] hook: GuardrailHookName) {
    assert_eq!(hook_from_name(name).unwrap(), hook);
}

#[test]
fn hook_names_reject_an_unknown_name() {
    let error = hook_from_name("pre_call").unwrap_err();
    assert_eq!(error.kind(), "InvalidHookName");
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
    let params = params_from_text(r#"{"guardrail": "pii", "mode": ["pre_call"]}"#).unwrap();
    assert_eq!(
        params.mode,
        LitellmParamsMode::Many(vec!["pre_call".to_owned()])
    );
    assert_eq!(params.guardrail, "pii");
    let error = params_from_text(r#"{"mode": "pre_call"}"#).unwrap_err();
    assert_eq!(error.kind(), "InvalidJson");
}

#[test]
fn record_input_reads_every_field() {
    let text = r#"{
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
    let input = record_input_from_text(text).unwrap();
    assert_eq!(input.guardrail_status, GuardrailStatus::GuardrailIntervened);
    assert_eq!(input.start_time, Some(10.0));
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
    let detail = input.tracing_detail.unwrap();
    assert_eq!(detail.guardrail_id.as_deref(), Some("g-1"));
    assert_eq!(detail.patterns_checked, Some(4));
    assert_eq!(detail.risk_score, Some(1.5));
    assert_eq!(
        detail.guardrail_cost.map(|cost| cost.to_string()),
        Some("1.50".to_owned())
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
    assert_eq!(inputs_from_text(None).unwrap(), None);
    assert_eq!(inputs_from_text(Some("null")).unwrap(), None);
    let inputs = inputs_from_text(Some(r#"{"texts": ["a"], "model": "gpt"}"#))
        .unwrap()
        .unwrap();
    assert_eq!(inputs.texts, Some(vec!["a".to_owned()]));
    assert_eq!(inputs.model.as_deref(), Some("gpt"));
    assert_eq!(
        inputs_from_text(Some("[]")).unwrap_err().kind(),
        "InvalidJson"
    );
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
fn record_parts_reject_an_unknown_context_key() {
    let error = record_parts(r#"{"settings": {}, "log": {}}"#)
        .err()
        .unwrap();
    assert_eq!(error.kind(), "InvalidJson");
    assert!(error.to_string().contains("log"), "{error}");
    assert!(record_parts("[]").is_err());
}
