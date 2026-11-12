use rstest::rstest;

use super::{base, dump, refused_field, text};
use crate::sections::guardrail_identity::enums::{PiiEntityType, Role};
use crate::sections::guardrail_identity::item::GuardrailValue;
use crate::sections::guardrail_params::base::BaseLitellmParams;
use crate::sections::guardrail_params::content_filter::ContentFilterConfigModel;
use crate::sections::guardrail_params::decode::{
    decode_apply_request, decode_litellm_params, decode_patch_request,
};
use crate::sections::guardrail_params::error::GuardrailParamsError;
use crate::sections::guardrail_params::opaque::OpaqueParams;
use crate::sections::guardrail_params::records::{
    DynamicGuardrailParams, Guardrail, GuardrailDefinitionLocation, GuardrailEventHooks,
    GuardrailItemSpec, LakeraCategoryThresholds, PiiEntityCategoryMap,
};
use crate::sections::guardrail_params::routes::{
    ApplyGuardrailResponse, GuardrailInfoResponse, GuardrailUiAddSettings, ListGuardrailsResponse,
};

#[test]
fn content_filter_round_trips_its_fields() {
    let body = r#"{"patterns": [{"pattern_type": "prebuilt", "pattern_name": "email", "action": "BLOCK"}, {"pattern_type": "regex", "pattern": "[0-9]+", "name": "digits", "action": "MASK"}], "blocked_words": [{"keyword": "secret", "action": "BLOCK", "description": "d"}], "blocked_words_file": "words.yaml", "categories": [{"category": "c", "enabled": false, "action": "MASK", "severity_threshold": "high", "category_file": "c.yaml"}], "severity_threshold": "low", "pattern_redaction_format": "[X]", "keyword_redaction_tag": "[Y]"}"#;
    let parsed = base(body).unwrap();
    assert_eq!(parsed.content_filter.patterns.as_ref().unwrap().0.len(), 2);
    assert_eq!(
        parsed.content_filter.blocked_words.as_ref().unwrap().len(),
        1
    );
    assert_eq!(parsed.content_filter.categories.as_ref().unwrap().len(), 1);
    let written = dump(&parsed);
    assert_eq!(written["blocked_words_file"], text("words.yaml"));
    assert_eq!(written["severity_threshold"], text("low"));
    assert_eq!(written["pattern_redaction_format"], text("[X]"));
    assert_eq!(written["keyword_redaction_tag"], text("[Y]"));
    let again = BaseLitellmParams::try_from(OpaqueParams(written.clone())).unwrap();
    assert_eq!(dump(&again), written);
}

#[rstest]
#[case::prebuilt("prebuilt", true)]
#[case::regex("regex", true)]
#[case::unknown("glob", false)]
#[case::uppercase("REGEX", false)]
fn content_filter_checks_the_pattern_type(#[case] pattern_type: &str, #[case] accepted: bool) {
    let body = format!(
        r#"{{"patterns": [{{"pattern_type": "{pattern_type}", "pattern_name": "n", "action": "BLOCK"}}]}}"#
    );
    let result = serde_json::from_str::<ContentFilterConfigModel>(&body);
    assert_eq!(result.is_ok(), accepted);
    if !accepted {
        assert_eq!(refused_field(base(&body)), "patterns");
    }
}

#[rstest]
#[case::pre_call(GuardrailEventHooks::PreCall, "pre_call")]
#[case::post_call(GuardrailEventHooks::PostCall, "post_call")]
#[case::during_call(GuardrailEventHooks::DuringCall, "during_call")]
#[case::logging_only(GuardrailEventHooks::LoggingOnly, "logging_only")]
#[case::pre_mcp_call(GuardrailEventHooks::PreMcpCall, "pre_mcp_call")]
#[case::during_mcp_call(GuardrailEventHooks::DuringMcpCall, "during_mcp_call")]
#[case::post_mcp_call(GuardrailEventHooks::PostMcpCall, "post_mcp_call")]
#[case::realtime(
    GuardrailEventHooks::RealtimeInputTranscription,
    "realtime_input_transcription"
)]
fn record_event_hooks_use_wire_names(#[case] hook: GuardrailEventHooks, #[case] wire: &str) {
    let quoted = format!("\"{wire}\"");
    assert_eq!(serde_json::to_string(&hook).unwrap(), quoted);
    assert_eq!(
        serde_json::from_str::<GuardrailEventHooks>(&quoted).unwrap(),
        hook
    );
}

#[test]
fn record_definition_location_defaults_to_config() {
    assert_eq!(
        GuardrailDefinitionLocation::default(),
        GuardrailDefinitionLocation::Config
    );
    assert_eq!(
        serde_json::to_string(&GuardrailDefinitionLocation::Db).unwrap(),
        "\"db\""
    );
    let info: GuardrailInfoResponse = serde_json::from_str(r#"{"guardrail_name": "g"}"#).unwrap();
    assert_eq!(
        info.guardrail_definition_location,
        GuardrailDefinitionLocation::Config
    );
    assert!(info.litellm_params.is_none());
}

#[test]
fn record_list_response_holds_info_with_base_params() {
    let list: ListGuardrailsResponse = serde_json::from_str(
        r#"{"guardrails": [{"guardrail_name": "g", "guardrail_definition_location": "db", "litellm_params": {"guard_name": "x", "top_p": 0.5}}]}"#,
    )
    .unwrap();
    let info = &list.guardrails[0];
    assert_eq!(
        info.guardrail_definition_location,
        GuardrailDefinitionLocation::Db
    );
    let held = info.litellm_params.as_ref().unwrap();
    assert_eq!(held.fields.guard_name.as_deref(), Some("x"));
    assert_eq!(held.extra.0["top_p"], GuardrailValue::Decimal(0.5));
}

#[test]
fn record_ui_settings_default_the_decision_providers() {
    let settings: GuardrailUiAddSettings = serde_json::from_str(
        r#"{"supported_entities": ["a"], "supported_actions": ["b"], "supported_modes": ["c"], "supported_modes_by_provider": {"p": ["pre_call"]}, "pii_entity_categories": [{"category": "k", "entities": ["e"]}]}"#,
    )
    .unwrap();
    assert!(settings.decision_model_providers.is_empty());
    assert!(settings.content_filter_settings.is_none());
    assert_eq!(settings.pii_entity_categories[0].category, "k");
}

#[test]
fn record_apply_request_defaults() {
    let request = decode_apply_request(br#"{"guardrail_name": "g", "text": "t"}"#).unwrap();
    assert_eq!(request.input_type, "request");
    assert!(request.entities.is_none());
    assert!(request.language.is_none());
}

#[test]
fn record_apply_request_reads_entities_and_ignores_unknown_keys() {
    let request = decode_apply_request(
        br#"{"guardrail_name": "g", "text": "t", "input_type": "response", "entities": ["EMAIL_ADDRESS", "PERSON"], "messages": [{"role": "user"}], "metadata": {"score": 0.5}, "unknown": true}"#,
    )
    .unwrap();
    assert_eq!(request.input_type, "response");
    assert_eq!(
        request.entities,
        Some(vec![PiiEntityType::EmailAddress, PiiEntityType::Person])
    );
    assert_eq!(request.messages.unwrap().len(), 1);
    assert_eq!(
        request.metadata.unwrap().0["score"],
        GuardrailValue::Decimal(0.5)
    );
}

#[rstest]
#[case::missing_text(br#"{"guardrail_name": "g"}"#.as_slice(), "")]
#[case::wrong_type(br#"{"guardrail_name": "g", "text": 5}"#.as_slice(), "text")]
#[case::unknown_entity(br#"{"guardrail_name": "g", "text": "t", "entities": ["NOPE"]}"#.as_slice(), "entities[0]")]
fn record_apply_request_errors_name_the_path(#[case] body: &[u8], #[case] path: &str) {
    let error = decode_apply_request(body).unwrap_err();
    if path.is_empty() {
        assert!(matches!(error, GuardrailParamsError::InvalidField { .. }));
    } else {
        assert!(matches!(error, GuardrailParamsError::InvalidField { field } if field == path));
    }
}

#[test]
fn record_apply_response_round_trips() {
    let response: ApplyGuardrailResponse =
        serde_json::from_str(r#"{"response_text": "ok"}"#).unwrap();
    assert_eq!(
        serde_json::to_string(&response).unwrap(),
        r#"{"response_text":"ok"}"#
    );
}

#[test]
fn record_patch_request_accepts_every_field_unset() {
    let request = decode_patch_request(b"{}").unwrap();
    assert!(request.guardrail_name.is_none());
    assert!(request.litellm_params.is_none());
    assert!(request.guardrail_info.is_none());
}

#[test]
fn record_patch_request_reads_base_params_and_ignores_unknown_keys() {
    let request = decode_patch_request(
        br#"{"guardrail_name": "n", "litellm_params": {"guard_name": "x", "api_base": null}, "guardrail_info": {"a": 1}, "unknown": 1}"#,
    )
    .unwrap();
    let held = request.litellm_params.unwrap();
    assert_eq!(held.fields.guard_name.as_deref(), Some("x"));
    assert_eq!(held.sent_fields().unwrap().len(), 2);
}

#[test]
fn record_patch_request_names_only_the_bad_field() {
    let error = decode_patch_request(br#"{"litellm_params": {"max_input_chars": 0}}"#).unwrap_err();
    assert!(
        matches!(error, GuardrailParamsError::InvalidField { field } if field == "litellm_params")
    );
}

#[rstest]
#[case::trailing(br#"{"guardrail": "g", "mode": "pre_call"} 1"#.as_slice())]
fn record_decoders_refuse_trailing_data(#[case] body: &[u8]) {
    assert!(matches!(
        decode_litellm_params(body),
        Err(GuardrailParamsError::TrailingData)
    ));
    assert!(matches!(
        decode_patch_request(body),
        Err(GuardrailParamsError::TrailingData)
    ));
    assert!(matches!(
        decode_apply_request(body),
        Err(GuardrailParamsError::TrailingData)
    ));
}

#[rstest]
#[case::truncated(br#"{"guardrail": "g", "mo"#.as_slice())]
#[case::not_utf8(b"\xff\xfe{".as_slice())]
#[case::wrong_root(b"[1]".as_slice())]
fn record_decoders_refuse_broken_bytes(#[case] body: &[u8]) {
    decode_litellm_params(body).unwrap_err();
    decode_patch_request(body).unwrap_err();
    decode_apply_request(body).unwrap_err();
}

#[test]
fn record_item_spec_needs_callbacks() {
    let spec: GuardrailItemSpec = serde_json::from_str(
        r#"{"callbacks": ["a"], "enabled_roles": ["user", "system"], "callback_args": {"a": {"k": 0.5}}}"#,
    )
    .unwrap();
    assert_eq!(spec.enabled_roles, Some(vec![Role::User, Role::System]));
    assert!(spec.default_on.is_none());
    assert!(spec.logging_only.is_none());
    assert_eq!(
        spec.callback_args.unwrap()["a"].0["k"],
        GuardrailValue::Decimal(0.5)
    );
    serde_json::from_str::<GuardrailItemSpec>(r#"{"default_on": true}"#).unwrap_err();
}

#[test]
fn record_small_types_keep_their_fields() {
    let map: PiiEntityCategoryMap =
        serde_json::from_str(r#"{"category": "c", "entities": ["a", "b"]}"#).unwrap();
    assert_eq!(map.entities, ["a", "b"]);
    let dynamic: DynamicGuardrailParams =
        serde_json::from_str(r#"{"extra_body": {"x": 1.5}}"#).unwrap();
    assert_eq!(dynamic.extra_body.0["x"], GuardrailValue::Decimal(1.5));
    let thresholds: LakeraCategoryThresholds =
        serde_json::from_str(r#"{"prompt_injection": 0.5, "jailbreak": 0.25}"#).unwrap();
    assert_eq!(thresholds.prompt_injection, Some(0.5_f64));
    assert_eq!(thresholds.jailbreak, Some(0.25_f64));
    assert_eq!(LakeraCategoryThresholds::default().jailbreak, None);
}

#[test]
fn record_guardrail_keeps_team_id() {
    let guardrail: Guardrail = serde_json::from_str(
        r#"{"guardrail_name": "g", "team_id": "t", "litellm_params": {"guardrail": "x", "mode": "pre_call"}}"#,
    )
    .unwrap();
    assert_eq!(guardrail.team_id.as_deref(), Some("t"));
    assert!(guardrail.guardrail_id.is_none());
    assert_eq!(dump(&guardrail)["team_id"], text("t"));
}
