#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::io::Write;
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use rstest::rstest;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::{LitellmParamsMode, Mode, ModeTarget};
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::GuardrailBase;
use super::advisory::{DEFAULT_ADVISORY_MESSAGE, inject_advisory_message};
use super::contract::GuardrailContract;
use super::error::GuardrailBaseError;
use super::failure::{GuardrailFailure, ModifyResponseFailure, SensitiveDataRouteFailure};
use super::hooks::validate_event_hook;
use super::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use super::mask::mask_content_in_string;
use super::session::session_id_from_request;
use super::settings::{GuardrailSettings, GuardrailSwitches};
use crate::generic_guardrail_contracts::GuardrailInputType;

type Body = BTreeMap<String, GuardrailValue>;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Ok(mut bytes) = self.0.lock() else {
            return Err(io::Error::other("capture buffer poisoned"));
        };
        bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn captured_warnings<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let captured = Captured::default();
    let sink = captured.clone();
    let subscriber = fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    let result = with_default(subscriber, run);
    let bytes = captured.0.lock().unwrap().clone();
    (result, String::from_utf8(bytes).unwrap())
}

fn body(json: &str) -> Body {
    serde_json::from_str(json).unwrap()
}

fn one(name: &str) -> LitellmParamsMode {
    LitellmParamsMode::One(name.to_owned())
}

fn many(names: &[&str]) -> LitellmParamsMode {
    LitellmParamsMode::Many(names.iter().map(|name| (*name).to_owned()).collect())
}

fn single(name: &str) -> ModeTarget {
    ModeTarget::One(name.to_owned())
}

fn list(names: &[&str]) -> ModeTarget {
    ModeTarget::Many(names.iter().map(|name| (*name).to_owned()).collect())
}

fn tagged(tags: &[(&str, ModeTarget)], default: Option<ModeTarget>) -> LitellmParamsMode {
    LitellmParamsMode::Tagged(Mode {
        tags: tags
            .iter()
            .map(|(tag, target)| ((*tag).to_owned(), target.clone()))
            .collect(),
        default,
    })
}

fn plain_switches() -> GuardrailSwitches {
    GuardrailSwitches {
        use_native_during_call_hook: false,
        use_native_lifecycle_hooks: false,
        records_own_information: false,
        uses_apply_guardrail_interface: false,
        deny_text_on_error: false,
    }
}

fn plain_settings() -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: None,
        supported_event_hooks: None,
        event_hook: None,
        default_on: false,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: plain_switches(),
    }
}

fn plain_inputs() -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: None,
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

fn interface_only() -> GuardrailSwitches {
    GuardrailSwitches {
        uses_apply_guardrail_interface: true,
        ..plain_switches()
    }
}

fn interface_and_native() -> GuardrailSwitches {
    GuardrailSwitches {
        use_native_lifecycle_hooks: true,
        uses_apply_guardrail_interface: true,
        ..plain_switches()
    }
}

fn native_only() -> GuardrailSwitches {
    GuardrailSwitches {
        use_native_lifecycle_hooks: true,
        ..plain_switches()
    }
}

fn unsupported(hook: &str, supported: &str) -> GuardrailBaseError {
    GuardrailBaseError::UnsupportedEventHook {
        hook: hook.to_owned(),
        supported: supported.to_owned(),
    }
}

fn unknown(hook: &str) -> GuardrailBaseError {
    GuardrailBaseError::UnknownEventHook {
        hook: hook.to_owned(),
    }
}

const PRE_AND_DURING: [GuardrailEventHooks; 2] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
];
const PRE_AND_DURING_TEXT: &str = "pre_call, during_call";

#[rstest]
#[case::missing_hook(None, Ok(()))]
#[case::supported_hook(Some(one("pre_call")), Ok(()))]
#[case::unsupported_hook(
    Some(one("post_call")),
    Err(unsupported("post_call", PRE_AND_DURING_TEXT))
)]
#[case::unknown_text(Some(one("after_call")), Err(unknown("after_call")))]
#[case::empty_text(Some(one("")), Err(unknown("")))]
#[case::upper_case_text(Some(one("PRE_CALL")), Err(unknown("PRE_CALL")))]
#[case::list_supported(Some(many(&["pre_call", "during_call"])), Ok(()))]
#[case::empty_list(Some(many(&[])), Ok(()))]
#[case::list_with_unsupported(Some(many(&["pre_call", "post_call"])), Err(unsupported("post_call", PRE_AND_DURING_TEXT)))]
#[case::list_with_unknown(Some(many(&["pre_call", "nope"])), Err(unknown("nope")))]
#[case::tag_supported(Some(tagged(&[("a", single("pre_call"))], None)), Ok(()))]
#[case::tag_list_supported(Some(tagged(&[("a", list(&["pre_call", "during_call"]))], None)), Ok(()))]
#[case::tag_unsupported(Some(tagged(&[("a", single("post_call"))], None)), Err(unsupported("post_call", PRE_AND_DURING_TEXT)))]
#[case::tag_list_unsupported(Some(tagged(&[("a", list(&["pre_call", "post_call"]))], None)), Err(unsupported("post_call", PRE_AND_DURING_TEXT)))]
#[case::tag_unknown(Some(tagged(&[("a", single("nope"))], None)), Err(unknown("nope")))]
#[case::tag_empty_text_is_checked(Some(tagged(&[("a", single(""))], None)), Err(unknown("")))]
#[case::no_tags(Some(tagged(&[], None)), Ok(()))]
#[case::default_supported(Some(tagged(&[("a", single("pre_call"))], Some(single("during_call")))), Ok(()))]
#[case::default_unsupported(Some(tagged(&[("a", single("pre_call"))], Some(single("post_call")))), Err(unsupported("post_call", PRE_AND_DURING_TEXT)))]
#[case::default_list_unsupported(Some(tagged(&[], Some(list(&["pre_call", "post_call"])))), Err(unsupported("post_call", PRE_AND_DURING_TEXT)))]
#[case::default_unknown(Some(tagged(&[], Some(single("nope")))), Err(unknown("nope")))]
#[case::default_empty_text_is_skipped(Some(tagged(&[("a", single("pre_call"))], Some(single("")))), Ok(()))]
#[case::default_empty_list_is_skipped(Some(tagged(&[("a", single("pre_call"))], Some(list(&[])))), Ok(()))]
fn validate_hook_checks_every_text_in_the_mode(
    #[case] event_hook: Option<LitellmParamsMode>,
    #[case] expected: Result<(), GuardrailBaseError>,
) {
    let result = validate_event_hook(
        event_hook.as_ref(),
        &PRE_AND_DURING,
        &GuardrailSwitches::default(),
    );
    assert_eq!(result, expected);
}

#[rstest]
#[case::apply_interface(interface_only(), true)]
#[case::no_apply_interface(GuardrailSwitches::default(), false)]
#[case::native_lifecycle(interface_and_native(), false)]
#[case::native_without_interface(native_only(), false)]
fn validate_hook_allows_logging_only_for_the_apply_interface(
    #[case] switches: GuardrailSwitches,
    #[case] allowed: bool,
) {
    let result = validate_event_hook(Some(&one("logging_only")), &PRE_AND_DURING, &switches);
    assert_eq!(result.is_ok(), allowed);
}

#[test]
fn validate_hook_allows_logging_only_inside_a_tagged_default() {
    let event_hook = tagged(&[], Some(single("logging_only")));
    let allowed = validate_event_hook(Some(&event_hook), &PRE_AND_DURING, &interface_only());
    let refused = validate_event_hook(
        Some(&event_hook),
        &PRE_AND_DURING,
        &GuardrailSwitches::default(),
    );
    assert_eq!(allowed, Ok(()));
    assert_eq!(
        refused,
        Err(unsupported("logging_only", PRE_AND_DURING_TEXT))
    );
}

#[test]
fn validate_hook_messages_follow_litellm() {
    let switches = interface_only();
    let refused = validate_event_hook(Some(&one("post_call")), &PRE_AND_DURING, &switches);
    assert_eq!(
        refused.unwrap_err().to_string(),
        "Event hook post_call is not in the supported event hooks [pre_call, during_call]"
    );
    let invalid = validate_event_hook(Some(&one("nope")), &PRE_AND_DURING, &switches);
    assert_eq!(
        invalid.unwrap_err().to_string(),
        "'nope' is not a valid GuardrailEventHooks"
    );
}

#[test]
fn validate_hook_rejects_any_hook_against_an_empty_supported_list() {
    let result = validate_event_hook(Some(&one("pre_call")), &[], &GuardrailSwitches::default());
    assert_eq!(result, Err(unsupported("pre_call", "")));
}

#[rstest]
#[case::pre_call(GuardrailEventHooks::PreCall, "pre_call", "post_call")]
#[case::post_call(GuardrailEventHooks::PostCall, "post_call", "pre_call")]
#[case::during_call(GuardrailEventHooks::DuringCall, "during_call", "pre_call")]
#[case::logging_only(GuardrailEventHooks::LoggingOnly, "logging_only", "pre_call")]
#[case::pre_mcp_call(GuardrailEventHooks::PreMcpCall, "pre_mcp_call", "pre_call")]
#[case::during_mcp_call(GuardrailEventHooks::DuringMcpCall, "during_mcp_call", "pre_call")]
#[case::post_mcp_call(GuardrailEventHooks::PostMcpCall, "post_mcp_call", "pre_call")]
#[case::realtime(
    GuardrailEventHooks::RealtimeInputTranscription,
    "realtime_input_transcription",
    "pre_call"
)]
fn validate_hook_prints_the_wire_name_of_each_supported_hook(
    #[case] supported: GuardrailEventHooks,
    #[case] wire: &str,
    #[case] requested: &str,
) {
    let result = validate_event_hook(
        Some(&one(requested)),
        &[supported],
        &GuardrailSwitches::default(),
    );
    assert_eq!(result, Err(unsupported(requested, wire)));
}

fn settings_with(
    supported: Option<Vec<GuardrailEventHooks>>,
    event_hook: LitellmParamsMode,
) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: Some("g".to_owned()),
        supported_event_hooks: supported,
        event_hook: Some(event_hook),
        ..plain_settings()
    }
}

#[test]
fn validate_hook_new_returns_the_error_when_strict() {
    let settings = settings_with(Some(PRE_AND_DURING.to_vec()), one("post_call"));
    let (result, warnings) = captured_warnings(|| GuardrailBase::new(settings, true));
    assert_eq!(
        result.unwrap_err(),
        unsupported("post_call", PRE_AND_DURING_TEXT)
    );
    assert_eq!(warnings, "");
}

#[test]
fn validate_hook_new_warns_once_and_continues_when_not_strict() {
    let settings = settings_with(Some(PRE_AND_DURING.to_vec()), one("post_call"));
    let (result, warnings) = captured_warnings(|| GuardrailBase::new(settings.clone(), false));
    let base = result.unwrap();
    assert_eq!(base.settings, settings);
    assert_eq!(base.mode, None);
    assert_eq!(warnings.lines().count(), 1);
    assert!(warnings.contains(
        "Event hook post_call is not in the supported event hooks [pre_call, during_call]. LITELLM_STRICT_GUARDRAIL_MODES=false; continuing with unsupported event_hook. Set the env var to true (default) to enforce validation and fail at startup."
    ));
}

#[rstest]
#[case::strict(true)]
#[case::lenient(false)]
fn validate_hook_new_accepts_a_supported_hook_without_a_warning(#[case] strict: bool) {
    let settings = settings_with(Some(PRE_AND_DURING.to_vec()), one("pre_call"));
    let (result, warnings) = captured_warnings(|| GuardrailBase::new(settings, strict));
    result.unwrap();
    assert_eq!(warnings, "");
}

#[rstest]
#[case::missing_list(None)]
#[case::empty_list(Some(Vec::new()))]
fn validate_hook_new_skips_validation_without_supported_hooks(
    #[case] supported: Option<Vec<GuardrailEventHooks>>,
) {
    let settings = settings_with(supported, one("not_a_hook"));
    let (result, warnings) = captured_warnings(|| GuardrailBase::new(settings, true));
    result.unwrap();
    assert_eq!(warnings, "");
}

#[test]
fn validate_hook_new_passes_the_switches_to_the_check() {
    let refused = settings_with(Some(PRE_AND_DURING.to_vec()), one("logging_only"));
    let allowed = GuardrailSettings {
        switches: interface_only(),
        ..refused.clone()
    };
    GuardrailBase::new(refused, true).unwrap_err();
    GuardrailBase::new(allowed, true).unwrap();
}

#[test]
fn contract_default_settings_have_sticky_routing_only() {
    let settings = GuardrailSettings::default();
    assert!(settings.sticky_session_routing);
    assert!(!settings.default_on);
    assert!(!settings.mask_request_content);
    assert!(!settings.mask_response_content);
    assert!(!settings.run_in_parallel);
    assert!(!settings.scan_raw_request);
    assert!(!settings.only_scan_new_messages);
    assert_eq!(settings.switches, GuardrailSwitches::default());
    assert_eq!(settings.guardrail_name, None);
    assert_eq!(settings.timeout, None);
}

#[test]
fn contract_new_keeps_the_timeout_as_given() {
    let settings = GuardrailSettings {
        timeout: Some(2.5_f64),
        ..plain_settings()
    };
    let base = GuardrailBase::new(settings, true).unwrap();
    assert_eq!(base.settings.timeout, Some(2.5_f64));
    assert_eq!(base.mode, None);
}

struct DefaultGuardrail;

impl GuardrailContract for DefaultGuardrail {}

#[tokio::test]
async fn contract_default_apply_returns_the_inputs_unchanged() {
    let inputs = GuardrailApiInputs {
        texts: Some(vec!["hello".to_owned()]),
        model: Some("gpt".to_owned()),
        stream_holdback_chars: Some(vec![3]),
        ..plain_inputs()
    };
    let mut request_data = body(r#"{"model": "gpt"}"#);
    let mut log_target = GuardrailLogTarget::default();
    let result = DefaultGuardrail
        .apply_guardrail(
            inputs.clone(),
            &mut request_data,
            GuardrailInputType::Request,
            Some(&mut log_target),
        )
        .await;
    assert_eq!(result.unwrap(), inputs);
    assert_eq!(request_data, body(r#"{"model": "gpt"}"#));
    assert_eq!(log_target, GuardrailLogTarget::default());
}

#[test]
fn contract_default_lookups_are_empty_and_scan_flags_follow_litellm() {
    assert_eq!(DefaultGuardrail.get_config_model(), None);
    assert_eq!(DefaultGuardrail.get_supported_event_hooks(), None);
    assert!(DefaultGuardrail.supports_tool_result_scan());
    assert!(!DefaultGuardrail.messages_cover_full_request());
}

#[test]
fn contract_inputs_write_only_the_fields_that_are_set() {
    let empty = serde_json::to_string(&GuardrailApiInputs::default()).unwrap();
    assert_eq!(empty, "{}");
    let full = r#"{"texts":["a"],"images":["data:image/png;base64,AA"],"tools":[{"type":"function","function":{"name":"f"}}],"tool_calls":[{"id":"call_1"}],"structured_messages":[{"role":"user","content":"hi"}],"model":"gpt","stream_holdback_chars":[1,2]}"#;
    let inputs: GuardrailApiInputs = serde_json::from_str(full).unwrap();
    assert_eq!(inputs.stream_holdback_chars, Some(vec![1, 2]));
    assert_eq!(serde_json::to_string(&inputs).unwrap(), full);
}

#[test]
fn contract_inputs_read_a_null_model_as_absent() {
    let inputs: GuardrailApiInputs =
        serde_json::from_str(r#"{"model": null, "texts": ["a"]}"#).unwrap();
    assert_eq!(inputs.model, None);
    assert_eq!(inputs.texts, Some(vec!["a".to_owned()]));
}

fn raised(name: Option<&str>, wrap_default_message: bool) -> GuardrailFailure {
    GuardrailFailure::GuardrailRaised {
        guardrail_name: name.map(str::to_owned),
        message: "blocked".to_owned(),
        wrap_default_message,
        status_code: 400,
        blocked_content: false,
    }
}

fn blocked_pii(name: Option<&str>) -> GuardrailFailure {
    GuardrailFailure::BlockedPii {
        entity_type: "EMAIL".to_owned(),
        guardrail_name: name.map(str::to_owned),
        status_code: 400,
    }
}

fn modify_response() -> GuardrailFailure {
    GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
        message: "replaced".to_owned(),
        model: "gpt".to_owned(),
        request_data: Body::new(),
        guardrail_name: None,
        detection_info: Body::new(),
        original_response: None,
    }))
}

fn route(message: Option<&str>) -> GuardrailFailure {
    GuardrailFailure::SensitiveDataRoute(Box::new(SensitiveDataRouteFailure {
        route_to_model: "onprem".to_owned(),
        session_id: "s1".to_owned(),
        guardrail_name: Some("pii".to_owned()),
        detection_info: Body::new(),
        message: message.map(str::to_owned),
        sticky_session_routing: true,
    }))
}

#[rstest]
#[case::wrapped(
    raised(Some("g"), true),
    "Guardrail raised an exception, Guardrail: g, Message: blocked"
)]
#[case::wrapped_without_name(
    raised(None, true),
    "Guardrail raised an exception, Guardrail: None, Message: blocked"
)]
#[case::unwrapped(raised(Some("g"), false), "blocked")]
#[case::pii(
    blocked_pii(Some("presidio")),
    "Blocked entity detected: EMAIL by Guardrail: presidio. This entity is not allowed to be used in this request."
)]
#[case::pii_without_name(
    blocked_pii(None),
    "Blocked entity detected: EMAIL by Guardrail: None. This entity is not allowed to be used in this request."
)]
#[case::modify_response(modify_response(), "replaced")]
#[case::route_given_message(route(Some("go private")), "go private")]
#[case::route_default_message(
    route(None),
    "Sensitive data detected by pii. Routing to model: onprem"
)]
#[case::http(GuardrailFailure::Http { status: 403, detail: "denied".to_owned() }, "denied")]
#[case::other(GuardrailFailure::Other("boom".to_owned()), "boom")]
fn failure_display_prints_the_exception_text(
    #[case] failure: GuardrailFailure,
    #[case] expected: &str,
) {
    assert_eq!(failure.to_string(), expected);
}

fn detail_object() -> IndexMap<String, GuardrailValue> {
    let mut detail = IndexMap::new();
    detail.insert("zebra".to_owned(), GuardrailValue::SignedInteger(1));
    detail.insert(
        "alpha".to_owned(),
        GuardrailValue::Array(vec![GuardrailValue::Boolean(true), GuardrailValue::Null]),
    );
    detail
}

#[test]
fn failure_http_object_keeps_key_order_in_display() {
    let failure = GuardrailFailure::HttpObject {
        status: 400,
        detail: detail_object(),
    };
    assert_eq!(failure.to_string(), r#"{"zebra":1,"alpha":[true,null]}"#);
}

#[rstest]
#[case::bad_request(400, true)]
#[case::forbidden(403, true)]
#[case::unprocessable(422, true)]
#[case::unauthorized(401, false)]
#[case::request_timeout(408, false)]
#[case::rate_limited(429, false)]
#[case::server_error(500, false)]
#[case::ok(200, false)]
fn failure_http_is_an_intervention_only_for_block_statuses(
    #[case] status: u16,
    #[case] expected: bool,
) {
    let plain = GuardrailFailure::Http {
        status,
        detail: String::new(),
    };
    let object = GuardrailFailure::HttpObject {
        status,
        detail: detail_object(),
    };
    assert_eq!(plain.is_intervention(), expected);
    assert_eq!(object.is_intervention(), expected);
}

#[rstest]
#[case::modify_response(modify_response(), true)]
#[case::raised(raised(Some("g"), true), true)]
#[case::pii(blocked_pii(None), true)]
#[case::route(route(None), true)]
#[case::other(GuardrailFailure::Other("boom".to_owned()), false)]
fn failure_is_intervention_for_the_guardrail_variants(
    #[case] failure: GuardrailFailure,
    #[case] expected: bool,
) {
    assert_eq!(failure.is_intervention(), expected);
}

fn base_with_template(template: Option<&str>, name: Option<&str>) -> GuardrailBase {
    GuardrailBase::new(
        GuardrailSettings {
            guardrail_name: name.map(str::to_owned),
            violation_message_template: template.map(str::to_owned),
            ..plain_settings()
        },
        true,
    )
    .unwrap()
}

#[rstest]
#[case::no_template(None, &[], "fallback")]
#[case::empty_template(Some(""), &[], "fallback")]
#[case::default_message(Some("Blocked: {default_message}"), &[], "Blocked: fallback")]
#[case::context_key(Some("{guardrail_name} said no"), &[("guardrail_name", "pii")], "pii said no")]
#[case::both_keys(Some("{guardrail_name}: {default_message}"), &[("guardrail_name", "pii")], "pii: fallback")]
#[case::context_overrides_default(Some("{default_message}"), &[("default_message", "mine")], "mine")]
#[case::later_context_wins(Some("{a}"), &[("a", "first"), ("a", "second")], "second")]
#[case::no_placeholders(Some("plain text"), &[], "plain text")]
#[case::escaped_braces(Some("{{kept}} {default_message}"), &[], "{kept} fallback")]
fn violation_renders_the_template(
    #[case] template: Option<&str>,
    #[case] context: &[(&str, &str)],
    #[case] expected: &str,
) {
    let base = base_with_template(template, Some("g"));
    let (rendered, warnings) =
        captured_warnings(|| base.render_violation_message("fallback", context));
    assert_eq!(rendered, expected);
    assert_eq!(warnings, "");
}

#[test]
fn violation_applies_a_width_spec_to_a_context_value() {
    let template = format!("[{open}a:>4}}]", open = '{');
    let base = base_with_template(Some(&template), Some("g"));
    let rendered = base.render_violation_message("fallback", &[("a", "x")]);
    assert_eq!(rendered, "[   x]");
}

#[rstest]
#[case::named(Some("g"), "for guardrail g: ")]
#[case::unnamed(None, "for guardrail None: ")]
fn violation_unknown_key_returns_the_default_and_warns(
    #[case] name: Option<&str>,
    #[case] expected_warning: &str,
) {
    let base = base_with_template(Some("{missing} blocked"), name);
    let (rendered, warnings) = captured_warnings(|| base.render_violation_message("fallback", &[]));
    assert_eq!(rendered, "fallback");
    assert_eq!(warnings.lines().count(), 1);
    assert!(warnings.contains(&format!(
        "Failed to format violation message template {expected_warning}"
    )));
}

#[test]
fn violation_unclosed_brace_returns_the_default() {
    let base = base_with_template(Some("{default_message"), Some("g"));
    let (rendered, warnings) = captured_warnings(|| base.render_violation_message("fallback", &[]));
    assert_eq!(rendered, "fallback");
    assert_eq!(warnings.lines().count(), 1);
}

fn passthrough(
    request_data: &Body,
    detection_info: Option<Body>,
    original_response: Option<GuardrailValue>,
) -> ModifyResponseFailure {
    let base = base_with_template(None, Some("g"));
    let failure = base.passthrough_failure("nope", request_data, detection_info, original_response);
    let GuardrailFailure::ModifyResponse(details) = failure else {
        panic!("expected ModifyResponse, got {failure:?}");
    };
    *details
}

#[test]
fn passthrough_carries_message_name_model_and_body() {
    let request_data = body(r#"{"model": "gpt", "messages": []}"#);
    let details = passthrough(&request_data, None, None);
    assert_eq!(details.message, "nope");
    assert_eq!(details.guardrail_name.as_deref(), Some("g"));
    assert_eq!(details.model, "gpt");
    assert_eq!(details.request_data, request_data);
    assert_eq!(details.detection_info, Body::new());
    assert_eq!(details.original_response, None);
}

#[rstest]
#[case::no_model_key(r#"{"messages": []}"#)]
#[case::null_model(r#"{"model": null}"#)]
#[case::numeric_model(r#"{"model": 5}"#)]
fn passthrough_uses_unknown_without_a_model_text(#[case] request_data: &str) {
    assert_eq!(
        passthrough(&body(request_data), None, None).model,
        "unknown"
    );
}

#[test]
fn passthrough_keeps_detection_info_and_the_original_response() {
    let detection_info = body(r#"{"score": 0.5}"#);
    let original_response = GuardrailValue::Object(body(r#"{"usage": {"total_tokens": 9}}"#));
    let details = passthrough(
        &Body::new(),
        Some(detection_info.clone()),
        Some(original_response.clone()),
    );
    assert_eq!(details.detection_info, detection_info);
    assert_eq!(details.original_response, Some(original_response));
}

fn injected(raw: &str) -> (bool, Body) {
    let mut data = body(raw);
    let landed = inject_advisory_message(&mut data, "careful");
    (landed, data)
}

fn with_advisory(expected: &str) -> Body {
    body(&expected.replace("ADVISORY", r#"{"role": "system", "content": "careful"}"#))
}

#[rstest]
#[case::instructions_with_messages(
    r#"{"instructions": "be brief", "messages": [{"role": "user", "content": "hi"}]}"#,
    true,
    r#"{"instructions": "be brief\n\ncareful", "messages": [{"role": "user", "content": "hi"}, ADVISORY]}"#
)]
#[case::instructions_alone(
    r#"{"instructions": "be brief"}"#,
    true,
    r#"{"instructions": "be brief\n\ncareful"}"#
)]
#[case::instructions_win_over_input(
    r#"{"instructions": "be brief", "input": "hello"}"#,
    true,
    r#"{"instructions": "be brief\n\ncareful", "input": "hello"}"#
)]
#[case::instructions_win_over_list_input(
    r#"{"instructions": "be brief", "input": [{"role": "user"}]}"#,
    true,
    r#"{"instructions": "be brief\n\ncareful", "input": [{"role": "user"}]}"#
)]
#[case::input_with_messages(
    r#"{"input": "hello", "messages": [{"role": "user", "content": "hi"}]}"#,
    true,
    r#"{"input": "hello\n\ncareful", "messages": [{"role": "user", "content": "hi"}, ADVISORY]}"#
)]
#[case::input_alone(r#"{"input": "hello"}"#, true, r#"{"input": "hello\n\ncareful"}"#)]
#[case::non_text_instructions_fall_to_input(
    r#"{"instructions": ["x"], "input": "hello"}"#,
    true,
    r#"{"instructions": ["x"], "input": "hello\n\ncareful"}"#
)]
#[case::list_input(
    r#"{"input": [{"role": "user"}]}"#,
    false,
    r#"{"input": [{"role": "user"}]}"#
)]
#[case::list_input_beats_messages(
    r#"{"input": [{"role": "user"}], "messages": [{"role": "user", "content": "hi"}]}"#,
    false,
    r#"{"input": [{"role": "user"}], "messages": [{"role": "user", "content": "hi"}]}"#
)]
#[case::number_input(r#"{"input": 5}"#, false, r#"{"input": 5}"#)]
#[case::null_input_with_messages(
    r#"{"input": null, "messages": []}"#,
    true,
    r#"{"input": null, "messages": [ADVISORY]}"#
)]
#[case::messages_list(
    r#"{"messages": [{"role": "user", "content": "hi"}]}"#,
    true,
    r#"{"messages": [{"role": "user", "content": "hi"}, ADVISORY]}"#
)]
#[case::no_messages(
    r#"{"model": "gpt"}"#,
    true,
    r#"{"model": "gpt", "messages": [ADVISORY]}"#
)]
#[case::messages_not_a_list(r#"{"messages": "hi"}"#, true, r#"{"messages": [ADVISORY]}"#)]
fn advisory_lands_in_the_first_field_the_model_reads(
    #[case] data: &str,
    #[case] landed: bool,
    #[case] expected: &str,
) {
    assert_eq!(injected(data), (landed, with_advisory(expected)));
}

#[test]
fn advisory_default_message_keeps_the_reason_placeholder() {
    assert_eq!(
        DEFAULT_ADVISORY_MESSAGE,
        "The user's latest message was flagged for {reason} by a content safety guardrail. This may be a false positive. Use your judgment: respond helpfully if the request is legitimate, or decline if it is not."
    );
}

#[rstest]
#[case::middle("hello world", "***", 2, 5, "he*** world")]
#[case::whole("abc", "X", 0, 3, "X")]
#[case::from_start("abc", "X", 0, 1, "Xbc")]
#[case::up_to_end("abc", "X", 2, 3, "abX")]
#[case::empty_mask("abc", "", 1, 2, "ac")]
#[case::multi_byte_inside("a\u{f1}b\u{20ac}c", "X", 1, 2, "aXb\u{20ac}c")]
#[case::multi_byte_tail("a\u{f1}b\u{20ac}c", "X", 2, 3, "a\u{f1}X\u{20ac}c")]
#[case::negative_start("abc", "X", -1, 2, "abc")]
#[case::negative_end("abc", "X", 0, -1, "abc")]
#[case::empty_range("abc", "X", 1, 1, "abc")]
#[case::reversed_range("abc", "X", 2, 1, "abc")]
#[case::end_past_length("abc", "X", 1, 4, "abc")]
#[case::far_past_length("abc", "X", 0, i64::MAX, "abc")]
#[case::empty_content("", "X", 0, 0, "")]
fn mask_replaces_the_code_point_range(
    #[case] content: &str,
    #[case] mask: &str,
    #[case] start: i64,
    #[case] end: i64,
    #[case] expected: &str,
) {
    assert_eq!(mask_content_in_string(content, mask, start, end), expected);
}

fn update_target() -> GuardrailBase {
    GuardrailBase::new(
        GuardrailSettings {
            guardrail_name: Some("kept".to_owned()),
            supported_event_hooks: Some(PRE_AND_DURING.to_vec()),
            event_hook: Some(one("pre_call")),
            default_on: true,
            mask_request_content: true,
            mask_response_content: true,
            violation_message_template: Some("old".to_owned()),
            end_session_after_n_fails: Some(3),
            on_violation: Some("warn".to_owned()),
            realtime_violation_message: Some("old".to_owned()),
            on_sensitive_data: Some("block".to_owned()),
            sensitive_data_route_to_model: Some("old".to_owned()),
            sticky_session_routing: false,
            run_in_parallel: true,
            scan_raw_request: true,
            only_scan_new_messages: true,
            timeout: Some(9.0_f64),
            switches: interface_only(),
        },
        true,
    )
    .unwrap()
}

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

#[test]
fn update_overwrites_the_listed_fields_with_the_params_defaults() {
    let mut base = update_target();
    base.update_params(&params(r#"{"guardrail": "g", "mode": "post_call"}"#));
    let settings = base.settings;
    assert!(!settings.default_on);
    assert!(!settings.mask_request_content);
    assert!(!settings.mask_response_content);
    assert_eq!(settings.violation_message_template, None);
    assert_eq!(settings.end_session_after_n_fails, None);
    assert_eq!(settings.on_violation, None);
    assert_eq!(settings.realtime_violation_message, None);
    assert_eq!(settings.on_sensitive_data, None);
    assert_eq!(settings.sensitive_data_route_to_model, None);
    assert!(settings.sticky_session_routing);
    assert!(!settings.run_in_parallel);
    assert!(!settings.scan_raw_request);
    assert!(!settings.only_scan_new_messages);
    assert_eq!(settings.timeout, None);
}

#[test]
fn update_copies_every_field_the_params_set() {
    let mut base = GuardrailBase::new(GuardrailSettings::default(), true).unwrap();
    base.update_params(&params(
        r#"{
            "guardrail": "g",
            "mode": "post_call",
            "default_on": true,
            "mask_request_content": true,
            "mask_response_content": true,
            "violation_message_template": "tpl",
            "end_session_after_n_fails": 4,
            "realtime_violation_message": "spoken",
            "sensitive_data_route_to_model": "onprem",
            "sticky_session_routing": false,
            "run_in_parallel": true,
            "scan_raw_request": true,
            "only_scan_new_messages": true,
            "timeout": 1.5
        }"#,
    ));
    let settings = base.settings;
    assert!(settings.default_on);
    assert!(settings.mask_request_content);
    assert!(settings.mask_response_content);
    assert_eq!(settings.violation_message_template.as_deref(), Some("tpl"));
    assert_eq!(settings.end_session_after_n_fails, Some(4));
    assert_eq!(
        settings.realtime_violation_message.as_deref(),
        Some("spoken")
    );
    assert_eq!(
        settings.sensitive_data_route_to_model.as_deref(),
        Some("onprem")
    );
    assert!(!settings.sticky_session_routing);
    assert!(settings.run_in_parallel);
    assert!(settings.scan_raw_request);
    assert!(settings.only_scan_new_messages);
    assert_eq!(settings.timeout, Some(1.5_f64));
}

#[rstest]
#[case::warn("warn")]
#[case::end_session("end_session")]
#[case::block("block")]
#[case::alert("alert")]
fn update_writes_the_on_violation_wire_name(#[case] wire: &str) {
    let mut base = update_target();
    base.update_params(&params(&format!(
        r#"{{"guardrail": "mcp_security", "mode": "pre_call", "on_violation": "{wire}"}}"#
    )));
    assert_eq!(base.settings.on_violation.as_deref(), Some(wire));
}

#[rstest]
#[case::block("block", "")]
#[case::route("route", r#", "sensitive_data_route_to_model": "onprem""#)]
fn update_writes_the_on_sensitive_data_wire_name(#[case] wire: &str, #[case] extra: &str) {
    let mut base = update_target();
    base.update_params(&params(&format!(
        r#"{{"guardrail": "g", "mode": "pre_call", "on_sensitive_data": "{wire}"{extra}}}"#
    )));
    assert_eq!(base.settings.on_sensitive_data.as_deref(), Some(wire));
}

#[test]
fn update_sets_mode_and_leaves_identity_fields_alone() {
    let mut base = update_target();
    let before = update_target();
    base.update_params(&params(
        r#"{"guardrail": "g", "mode": ["post_call", "during_call"]}"#,
    ));
    assert_eq!(base.mode, Some(many(&["post_call", "during_call"])));
    assert_eq!(base.settings.guardrail_name, before.settings.guardrail_name);
    assert_eq!(base.settings.event_hook, Some(one("pre_call")));
    assert_eq!(
        base.settings.supported_event_hooks,
        before.settings.supported_event_hooks
    );
    assert_eq!(base.settings.switches, before.settings.switches);
}

#[rstest]
#[case::top_level(r#"{"litellm_session_id": "top", "metadata": {"session_id": "meta"}, "litellm_metadata": {"session_id": "lm"}}"#, Some("top"))]
#[case::metadata(
    r#"{"metadata": {"session_id": "meta"}, "litellm_metadata": {"session_id": "lm"}}"#,
    Some("meta")
)]
#[case::litellm_metadata(r#"{"litellm_metadata": {"session_id": "lm"}}"#, Some("lm"))]
#[case::empty_top_level_text(
    r#"{"litellm_session_id": "", "metadata": {"session_id": "meta"}}"#,
    Some("meta")
)]
#[case::null_top_level(
    r#"{"litellm_session_id": null, "metadata": {"session_id": "meta"}}"#,
    Some("meta")
)]
#[case::empty_metadata_text(
    r#"{"metadata": {"session_id": ""}, "litellm_metadata": {"session_id": "lm"}}"#,
    Some("lm")
)]
#[case::null_metadata_value(
    r#"{"metadata": {"session_id": null}, "litellm_metadata": {"session_id": "lm"}}"#,
    Some("lm")
)]
#[case::null_metadata(
    r#"{"metadata": null, "litellm_metadata": {"session_id": "lm"}}"#,
    Some("lm")
)]
#[case::text_metadata(
    r#"{"metadata": "x", "litellm_metadata": {"session_id": "lm"}}"#,
    Some("lm")
)]
#[case::metadata_without_session(
    r#"{"metadata": {"other": "x"}, "litellm_metadata": {"session_id": "lm"}}"#,
    Some("lm")
)]
#[case::integer_top_level(r#"{"litellm_session_id": 42}"#, Some("42"))]
#[case::negative_integer(r#"{"metadata": {"session_id": -7}}"#, Some("-7"))]
#[case::huge_integer(
    r#"{"metadata": {"session_id": 18446744073709551615}}"#,
    Some("18446744073709551615")
)]
#[case::boolean(
    r#"{"litellm_session_id": true, "metadata": {"session_id": "meta"}}"#,
    Some("meta")
)]
#[case::nothing(r#"{"model": "gpt"}"#, None)]
#[case::empty_buckets(r#"{"metadata": {}, "litellm_metadata": {}}"#, None)]
fn session_id_follows_the_three_sources_in_order(
    #[case] data: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(session_id_from_request(&body(data)).as_deref(), expected);
}
