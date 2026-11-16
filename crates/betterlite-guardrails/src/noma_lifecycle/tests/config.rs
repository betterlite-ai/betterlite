use std::time::Duration;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use rstest::rstest;
use secrecy::ExposeSecret;
use tracing::Level;
use tracing::subscriber::with_default;

use super::support::{CapturedLog, build, build_with, environment, hooked, params};
use crate::noma_lifecycle::config::NomaParams;
use crate::noma_lifecycle::error::NomaLifecycleError;
use crate::noma_lifecycle::{ConfigModel, SUPPORTED_EVENT_HOOKS};

#[test]
fn supported_event_hooks_are_pre_during_post_and_pre_mcp() {
    let expected = [
        GuardrailEventHooks::PreCall,
        GuardrailEventHooks::DuringCall,
        GuardrailEventHooks::PostCall,
        GuardrailEventHooks::PreMcpCall,
    ];
    assert_eq!(SUPPORTED_EVENT_HOOKS, expected);
    let guardrail = build(&params("http://noma"));
    assert_eq!(
        guardrail.base.settings.supported_event_hooks,
        Some(expected.to_vec())
    );
}

#[test]
fn get_config_model_is_the_noma_guardrail_config_model() {
    assert_eq!(ConfigModel::ui_friendly_name(), "Noma Security");
}

#[test]
fn constructor_values_win_over_the_environment() {
    let env = environment(&[
        ("NOMA_API_KEY", "env-key"),
        ("NOMA_API_BASE", "http://env-base"),
        ("NOMA_APPLICATION_ID", "env-app"),
    ]);
    let guardrail = build_with(&params("http://param-base"), &env).unwrap();
    let settings = &guardrail.policy.settings;
    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("test-api-key")
    );
    assert_eq!(settings.api_base.as_deref(), Some("http://param-base"));
    assert_eq!(settings.application_id.as_deref(), Some("test-app"));
}

#[test]
fn the_environment_fills_what_the_constructor_leaves_out() {
    let env = environment(&[
        ("NOMA_API_KEY", "env-key"),
        ("NOMA_API_BASE", "http://env-base"),
        ("NOMA_APPLICATION_ID", "env-app"),
    ]);
    let guardrail = build_with(&NomaParams::default(), &env).unwrap();
    let settings = &guardrail.policy.settings;
    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("env-key")
    );
    assert_eq!(settings.api_base.as_deref(), Some("http://env-base"));
    assert_eq!(settings.application_id.as_deref(), Some("env-app"));
}

#[rstest]
#[case::unset(&[])]
#[case::empty(&[("NOMA_API_KEY", ""), ("NOMA_API_BASE", ""), ("NOMA_APPLICATION_ID", "")])]
fn unset_and_empty_environment_values_leave_the_defaults(#[case] entries: &[(&str, &str)]) {
    let guardrail = build_with(&NomaParams::default(), &environment(entries)).unwrap();
    let settings = &guardrail.policy.settings;
    assert!(settings.api_key.is_none());
    assert_eq!(
        settings.api_base.as_deref(),
        Some("https://api.noma.security/")
    );
    assert!(settings.application_id.is_none());
}

#[rstest]
#[case::nothing_set(&[], false, true, false)]
#[case::env_true(
    &[("NOMA_MONITOR_MODE", "true"), ("NOMA_ANONYMIZE_INPUT", "true")],
    true,
    true,
    true
)]
#[case::env_upper_case(&[("NOMA_MONITOR_MODE", "TRUE")], true, true, false)]
#[case::env_block_failures_false(&[("NOMA_BLOCK_FAILURES", "false")], false, false, false)]
#[case::empty_env_is_unset(
    &[("NOMA_MONITOR_MODE", ""), ("NOMA_BLOCK_FAILURES", ""), ("NOMA_ANONYMIZE_INPUT", "")],
    false,
    true,
    false
)]
fn flags_default_from_the_environment(
    #[case] entries: &[(&str, &str)],
    #[case] monitor_mode: bool,
    #[case] block_failures: bool,
    #[case] anonymize_input: bool,
) {
    let guardrail = build_with(&NomaParams::default(), &environment(entries)).unwrap();
    assert_eq!(guardrail.policy.settings.monitor_mode, monitor_mode);
    assert_eq!(guardrail.block_failures, block_failures);
    assert_eq!(guardrail.policy.settings.anonymize_input, anonymize_input);
}

#[test]
fn constructor_flags_win_over_the_environment() {
    let env = environment(&[
        ("NOMA_MONITOR_MODE", "false"),
        ("NOMA_BLOCK_FAILURES", "true"),
        ("NOMA_ANONYMIZE_INPUT", "false"),
    ]);
    let mut configured = params("http://noma");
    configured.monitor_mode = Some(true);
    configured.block_failures = Some(false);
    configured.anonymize_input = Some(true);
    let guardrail = build_with(&configured, &env).unwrap();
    assert!(guardrail.policy.settings.monitor_mode);
    assert!(!guardrail.block_failures);
    assert!(guardrail.policy.settings.anonymize_input);
}

#[rstest]
#[case::monitor("NOMA_MONITOR_MODE")]
#[case::block_failures("NOMA_BLOCK_FAILURES")]
#[case::anonymize("NOMA_ANONYMIZE_INPUT")]
fn a_flag_that_is_no_boolean_is_an_error(#[case] name: &str) {
    let error = build_with(&NomaParams::default(), &environment(&[(name, "maybe")])).unwrap_err();
    assert!(matches!(error, NomaLifecycleError::Environment(_)));
}

#[test]
fn the_deprecation_warning_is_emitted_once_per_process() {
    let log = CapturedLog::default();
    with_default(log.subscriber(Level::WARN), || {
        build(&params("http://noma"));
        build(&params("http://noma"));
        build(&NomaParams::default());
    });
    let text = log.text();
    assert_eq!(
        text.matches("Guardrail provider 'noma' is deprecated")
            .count(),
        1
    );
    assert!(text.contains("Please migrate to 'noma_v2'"));
}

#[rstest]
#[case::seconds(2.5, Some(Duration::from_millis(2500)))]
#[case::zero(0.0, Some(Duration::ZERO))]
#[case::negative(-1.0, None)]
#[case::not_a_number(f64::NAN, None)]
fn the_timeout_becomes_a_duration(#[case] seconds: f64, #[case] expected: Option<Duration>) {
    let mut configured = params("http://noma");
    configured.timeout = Some(seconds);
    let built = build_with(&configured, &environment(&[]));
    match expected {
        Some(timeout) => {
            assert_eq!(built.unwrap().policy.settings.timeout, Some(timeout));
        }
        None => assert!(matches!(
            built.unwrap_err(),
            NomaLifecycleError::Timeout { .. }
        )),
    }
}

#[rstest]
#[case::pre_call("pre_call", true)]
#[case::during_call("during_call", true)]
#[case::post_call("post_call", true)]
#[case::pre_mcp_call("pre_mcp_call", true)]
#[case::logging_only("logging_only", false)]
#[case::during_mcp_call("during_mcp_call", false)]
fn the_event_hook_must_be_a_supported_one(#[case] hook: &str, #[case] accepted: bool) {
    let built = build_with(&hooked("http://noma", hook), &environment(&[]));
    assert_eq!(built.is_ok(), accepted);
    if !accepted {
        assert!(matches!(
            built.unwrap_err(),
            NomaLifecycleError::Base { .. }
        ));
    }
}
