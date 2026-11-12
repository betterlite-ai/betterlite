use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{load, minimal, text};
use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::guardrail_params::base::UnreachableFallback;
use crate::sections::guardrail_params::decision_check::GuardrailChecks;
use crate::sections::guardrail_params::keys::{BASE_KEYS, PARAM_KEYS};
use crate::sections::guardrail_params::mode::LitellmParamsMode;

#[test]
fn section_load_reads_a_pre_call_entry() {
    let settings = load(&minimal(""), &[]);
    assert!(settings.rejected.is_empty());
    let guardrail = &settings.guardrails[0];
    assert_eq!(guardrail.guardrail_name, "safety");
    assert_eq!(guardrail.litellm_params.guardrail, "custom");
    assert_eq!(
        guardrail.litellm_params.mode,
        LitellmParamsMode::One("pre_call".to_owned())
    );
    assert_eq!(guardrail.team_id, None);
    assert!(guardrail.litellm_params.base.extra.0.is_empty());
}

#[test]
fn section_load_keeps_file_order_and_entry_fields() {
    let source = "guardrails:\n  - guardrail_name: first\n    guardrail_id: id-1\n    policy_template: tpl\n    guardrail_info: {owner: team}\n    created_at: \"2025-01-01T00:00:00Z\"\n    litellm_params: {guardrail: custom, mode: [pre_call, post_call]}\n  - guardrail_name: second\n    litellm_params: {guardrail: custom, mode: during_call, default_on: true}\n";
    let settings = load(source, &[]);
    let names: Vec<&str> = settings
        .guardrails
        .iter()
        .map(|item| item.guardrail_name.as_str())
        .collect();
    assert_eq!(names, ["first", "second"]);
    let first = &settings.guardrails[0];
    assert_eq!(first.guardrail_id.as_deref(), Some("id-1"));
    assert_eq!(first.policy_template.as_deref(), Some("tpl"));
    assert_eq!(
        first.guardrail_info.as_ref().unwrap().0["owner"],
        text("team")
    );
    assert!(first.created_at.is_some());
    assert!(first.updated_at.is_none());
    assert_eq!(first.litellm_params.base.fields.default_on, Some(false));
    assert_eq!(
        settings.guardrails[1].litellm_params.base.fields.default_on,
        Some(true)
    );
}

#[test]
fn section_load_without_guardrails_is_empty() {
    let settings = load("model_list: []\n", &[]);
    assert!(settings.guardrails.is_empty());
    assert!(settings.rejected.is_empty());
}

const EVERY_KEY: &str = r#"guardrails:
  - guardrail_name: everything
    litellm_params:
      guardrail: decision_model
      mode: pre_call
      decision_model: gpt-4o
      checks:
        - name: no-secrets
          instructions: Block secrets
      patterns:
        - {pattern_type: prebuilt, pattern_name: email, action: BLOCK}
      blocked_words:
        - {keyword: secret, action: BLOCK}
      blocked_words_file: words.yaml
      categories:
        - {category: harmful, action: BLOCK}
      severity_threshold: medium
      pattern_redaction_format: "[R]"
      keyword_redaction_tag: "[K]"
      api_key: sk-test
      api_base: https://example.test
      experimental_use_latest_role_message_only: true
      only_scan_new_messages: true
      skip_system_message_in_guardrail: true
      skip_tool_message_in_guardrail: true
      scan_only_tool_results: true
      inspect_embeddings: true
      category_thresholds: {prompt_injection: 0.5, jailbreak: 0.4}
      detect_secrets_config: {plugins: base64}
      guard_name: guard
      default_on: true
      mask_request_content: true
      mask_response_content: true
      pangea_input_recipe: input
      pangea_output_recipe: output
      model: gpt-4o
      violation_message_template: violated
      end_session_after_n_fails: 3
      on_violation: warn
      realtime_violation_message: stopped
      template_id: template
      location: us
      credentials: cred
      api_endpoint: https://endpoint.test
      fail_on_error: false
      skip_unscannable_attachments: true
      sanitize_error_detail: false
      additional_provider_specific_params: {region: eu}
      unreachable_fallback: fail_open
      max_input_chars: 100
      max_concurrent_decision_calls: 2
      extra_headers: [x-one]
      custom_code: "def apply(): pass"
      timeout: 2.5
      on_sensitive_data: route
      sensitive_data_route_to_model: gpt-4o-mini
      sticky_session_routing: false
      run_in_parallel: true
      scan_raw_request: true
"#;

#[test]
fn section_load_every_declared_key() {
    let mentioned = BASE_KEYS
        .iter()
        .chain(PARAM_KEYS.iter())
        .all(|key| EVERY_KEY.contains(&format!("      {key}:")));
    assert!(mentioned);
    let settings = load(EVERY_KEY, &[]);
    assert!(settings.rejected.is_empty(), "{:?}", settings.rejected);
    let held = &settings.guardrails[0].litellm_params;
    assert_eq!(held.base.fields.timeout, Some(2.5_f64));
    assert_eq!(
        held.base.fields.unreachable_fallback,
        UnreachableFallback::FailOpen
    );
    assert_eq!(
        held.base.fields.api_key.as_ref().unwrap().expose_secret(),
        "sk-test"
    );
    assert!(!held.base.handling_flags.sticky_session_routing);
    assert!(matches!(held.checks, Some(GuardrailChecks::Decision(_))));
}

#[test]
fn section_reject_missing_mode_keeps_the_load_going() {
    let source = "guardrails:\n  - guardrail_name: good\n    litellm_params: {guardrail: custom, mode: pre_call}\n  - guardrail_name: no-mode\n    litellm_params: {guardrail: custom}\n  - guardrail_name: also-good\n    litellm_params: {guardrail: custom, mode: post_call}\n";
    let settings = load(source, &[]);
    let names: Vec<&str> = settings
        .guardrails
        .iter()
        .map(|item| item.guardrail_name.as_str())
        .collect();
    assert_eq!(names, ["good", "also-good"]);
    assert_eq!(settings.rejected.len(), 1);
    assert_eq!(settings.rejected[0].guardrail_name, "no-mode");
    assert_eq!(settings.rejected[0].reason, "missing field mode");
}

#[rstest]
#[case::zero_limit("      max_input_chars: 0\n", "invalid field max_input_chars")]
#[case::block_elsewhere(
    "      on_violation: block\n",
    "on_violation='block' is only supported by guardrail='mcp_security'"
)]
#[case::bad_timeout("      timeout: leak-secret\n", "timeout must be numeric")]
#[case::route_without_model(
    "      on_sensitive_data: route\n",
    "sensitive_data_route_to_model must be set when on_sensitive_data='route'"
)]
fn section_reject_gives_a_reason_without_the_value(#[case] extra: &str, #[case] reason: &str) {
    let settings = load(&minimal(extra), &[]);
    assert!(settings.guardrails.is_empty());
    assert_eq!(settings.rejected[0].reason, reason);
    assert_eq!(settings.rejected[0].guardrail_name, "safety");
}

#[test]
fn section_reject_missing_guardrail_is_recorded() {
    let source = "guardrails:\n  - guardrail_name: bare\n    litellm_params: {mode: pre_call}\n";
    let settings = load(source, &[]);
    assert_eq!(settings.rejected[0].reason, "missing field guardrail");
}

#[test]
fn section_unknown_litellm_param_fails_the_load() {
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&minimal("      misspelled: true\n"))
    .unwrap_err();
    assert!(matches!(error, ConfigError::Unsupported { paths }
        if paths == ["guardrails.0.litellm_params.misspelled"]));
}

#[test]
fn section_unknown_entry_key_fails_the_load() {
    let source = format!("{}    surprise: 1\n", minimal(""));
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .unwrap_err();
    assert!(matches!(error, ConfigError::Unsupported { paths }
        if paths == ["guardrails.0.surprise"]));
}
