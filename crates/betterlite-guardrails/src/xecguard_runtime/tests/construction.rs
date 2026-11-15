use std::time::Duration;

use betterlite_config::sections::xecguard_runtime::Settings as RuntimeSettings;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract as _;

use super::super::XecGuardGuard;
use super::support::{
    GUARDRAIL_NAME, SAFE, USER_REQUEST, args, base, client, default_policies, environment, field,
    inputs, mock, new_guard, request_data, seen, text, texts,
};

#[rstest]
#[case::true_text("true", true)]
#[case::one_text("1", true)]
#[case::yes_text("yes", true)]
#[case::upper_case("TRUE", true)]
#[case::mixed_case("Yes", true)]
#[case::false_text("false", false)]
#[case::zero_text("0", false)]
#[case::on_text("on", false)]
#[case::other_text("anything", false)]
fn xecguard_runtime_block_on_error_text(#[case] text: &str, #[case] expected: bool) {
    let mut environment = environment();
    environment.block_on_error = Some(text.to_owned());
    let guard = new_guard(args(), &environment, base(None, None));
    assert_eq!(guard.settings.block_on_error, expected);
}

pub(super) fn constructor_precedence() {
    let mut input = args();
    input.api_base = Some("http://args.test/".to_owned());
    input.xecguard_model = Some("model-from-args".to_owned());
    input.policy_names = Some(vec!["Custom-Policy".to_owned()]);
    input.block_on_error = Some(true);
    input.grounding_strictness = Some("STRICT".to_owned());
    let guard = new_guard(
        &input,
        &environment(),
        base(Some(GUARDRAIL_NAME), Some(2.5_f64)),
    );
    assert_eq!(guard.settings.api_key.expose_secret(), "key-from-args");
    assert_eq!(guard.settings.api_base, "http://args.test");
    assert_eq!(guard.settings.model, "model-from-args");
    assert_eq!(
        guard.settings.policy_names,
        vec!["Custom-Policy".to_owned()]
    );
    assert!(guard.settings.block_on_error);
    assert_eq!(guard.settings.grounding_strictness, "STRICT");
    assert_eq!(guard.settings.timeout, Duration::from_millis(2500));
    assert_eq!(
        guard.base.settings.guardrail_name.as_deref(),
        Some(GUARDRAIL_NAME)
    );
}

pub(super) fn environment_fallbacks() {
    let mut input = args();
    input.api_key = None;
    input.api_base = None;
    input.policy_names = Some(Vec::new());
    let guard = new_guard(&input, &environment(), base(None, None));
    assert_eq!(
        guard.settings.api_key.expose_secret(),
        "key-from-environment"
    );
    assert_eq!(guard.settings.api_base, "http://from-environment.test");
    assert_eq!(guard.settings.model, "xecguard_v2");
    assert!(!guard.settings.block_on_error);
    assert_eq!(guard.settings.grounding_strictness, "BALANCED");
    assert_eq!(guard.settings.timeout, Duration::from_secs(10));
    assert_eq!(guard.settings.policy_names, default_policies());
}

pub(super) fn unset_defaults() {
    let guard = new_guard(args(), &RuntimeSettings::default(), base(None, None));
    assert_eq!(guard.settings.api_base, "https://api-xecguard.cycraft.ai");
    assert_eq!(guard.settings.model, "xecguard_v2");
    assert!(guard.settings.block_on_error);
    assert_eq!(guard.settings.grounding_strictness, "BALANCED");
    assert_eq!(guard.settings.timeout, Duration::from_secs(10));
    assert_eq!(guard.settings.policy_names, default_policies());
}

pub(super) fn missing_key() {
    let mut input = args();
    input.api_key = None;
    let error = XecGuardGuard::new(
        &input,
        &RuntimeSettings::default(),
        base(None, None),
        client(),
    )
    .expect_err("a key is required");
    assert_eq!(
        error.to_string(),
        "XecGuard API key is required. Set XECGUARD_API_KEY in the environment or pass api_key in the guardrail config."
    );
}

pub(super) async fn default_policy_list() {
    let mock = mock((200, SAFE), (200, SAFE));
    let mut input = args();
    input.api_base = Some(mock.base.clone());
    input.policy_names = Some(Vec::new());
    let guard = new_guard(&input, &RuntimeSettings::default(), base(None, None));
    let mut data = request_data(USER_REQUEST);
    let returned = guard
        .apply_guardrail(
            inputs(&["hello"]),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("a pass");
    assert_eq!(returned, inputs(&["hello"]));
    let seen = seen(&mock);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/xecguard/v1/scan");
    assert_eq!(
        texts(field(&seen[0].body, "policy_names")),
        default_policies()
    );
    assert_eq!(text(field(&seen[0].body, "scan_type")), "input");
}

pub(super) async fn configured_policy_list() {
    let mock = mock((200, SAFE), (200, SAFE));
    let mut input = args();
    input.api_base = Some(mock.base.clone());
    input.policy_names = Some(vec!["Custom-Policy".to_owned()]);
    let guard = new_guard(&input, &RuntimeSettings::default(), base(None, None));
    let mut data = request_data(USER_REQUEST);
    guard
        .apply_guardrail(
            inputs(&["hello"]),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("a pass");
    let seen = seen(&mock);
    assert_eq!(seen.len(), 1);
    assert_eq!(
        texts(field(&seen[0].body, "policy_names")),
        vec!["Custom-Policy".to_owned()]
    );
}

pub(super) fn unrepresentable_timeout() {
    let error = XecGuardGuard::new(
        &args(),
        &RuntimeSettings::default(),
        base(Some(GUARDRAIL_NAME), Some(-1.0_f64)),
        client(),
    )
    .expect_err("a negative timeout is not a duration");
    assert_eq!(
        error.to_string(),
        "the XecGuard timeout is not a representable duration"
    );
}
