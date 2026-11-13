use std::sync::Arc;

use rstest::rstest;

use super::fake::ALLOW_SOURCE;
use super::fake::FakeHost;
use super::fake::LoadReply;
use super::fake::as_host;
use super::fake::build;
use super::fake::fixed_clock;
use super::fake::object;
use super::fake::params;
use super::fake::params_with_timeout;
use super::fake::text;
use super::fake::value;
use crate::custom_code::CustomCodeGuardrail;
use crate::custom_code::config::CustomCodeGuardrailParams;
use crate::custom_code::config::DEFAULT_GUARDRAIL_NAME;
use crate::custom_code::error::CustomCodeGuardrailError;

fn host_with_allow() -> Arc<FakeHost> {
    FakeHost::new(LoadReply::Module(value(object(vec![(
        "action",
        text("allow"),
    )]))))
}

#[test]
fn the_constructor_defaults_the_name_to_custom_code_and_the_timeout_to_thirty() {
    let host = host_with_allow();
    let guardrail = build(params(ALLOW_SOURCE), &host);
    assert_eq!(guardrail.guardrail_name, DEFAULT_GUARDRAIL_NAME);
    assert_eq!(guardrail.execution_timeout.to_bits(), 30.0_f64.to_bits());
}

#[test]
fn the_constructor_keeps_the_given_name_and_timeout() {
    let host = host_with_allow();
    let mut configured = params_with_timeout(ALLOW_SOURCE, 2.5);
    configured.guardrail_name = Some("my_guard".to_owned());
    let guardrail = build(configured, &host);
    assert_eq!(guardrail.guardrail_name, "my_guard");
    assert_eq!(guardrail.execution_timeout.to_bits(), 2.5_f64.to_bits());
}

#[test]
fn the_constructor_compiles_the_code_and_builds_the_compile_lock() {
    let host = host_with_allow();
    let guardrail = build(params(ALLOW_SOURCE), &host);
    assert_eq!(host.load_count(), 1);
    let state = guardrail.compile_lock.lock().unwrap();
    assert!(state.code.is_some());
    assert!(state.error.is_none());
    drop(state);
}

#[test]
fn the_constructor_refuses_a_missing_custom_code() {
    let host = host_with_allow();
    let configured = CustomCodeGuardrailParams {
        custom_code: None,
        guardrail_name: Some("custom_code".to_owned()),
        timeout: None,
    };
    let Err(error) = CustomCodeGuardrail::new(configured, as_host(&host), fixed_clock) else {
        panic!("expected the missing custom code error");
    };
    assert!(matches!(error, CustomCodeGuardrailError::MissingCustomCode));
    assert_eq!(host.load_count(), 0);
}

#[rstest]
#[case::zero(0.0_f64)]
#[case::negative(-3.0_f64)]
fn the_constructor_refuses_a_non_positive_timeout(#[case] timeout: f64) {
    let host = host_with_allow();
    let Err(error) = CustomCodeGuardrail::new(
        params_with_timeout(ALLOW_SOURCE, timeout),
        as_host(&host),
        fixed_clock,
    ) else {
        panic!("expected the invalid timeout error");
    };
    assert!(matches!(
        error,
        CustomCodeGuardrailError::NonPositiveTimeout { .. }
    ));
    assert!(error.to_string().contains("must be positive"));
    assert_eq!(host.load_count(), 0);
}
