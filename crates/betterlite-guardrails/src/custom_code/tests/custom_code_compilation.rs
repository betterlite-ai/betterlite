use std::sync::Arc;

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
use crate::custom_code::error::CustomCodeGuardrailError;

const LOOP_SOURCE: &str = "def apply_guardrail(inputs, request_data, input_type):\n    while running:\n        return allow()\n";

fn loaded_host() -> Arc<FakeHost> {
    FakeHost::new(LoadReply::Module(value(object(vec![(
        "action",
        text("allow"),
    )]))))
}

#[test]
fn compiling_twice_reuses_the_cached_module() {
    let host = loaded_host();
    let guardrail = build(params(ALLOW_SOURCE), &host);
    let first = guardrail.compile_custom_code().unwrap();
    let second = guardrail.compile_custom_code().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(host.load_count(), 1);
}

#[test]
fn the_guardrail_compiles_with_the_sandbox_and_passes_the_globals_to_the_host() {
    let host = loaded_host();
    let _guardrail = build(params(LOOP_SOURCE), &host);
    let sources = host.sources.lock().unwrap();
    assert_eq!(sources.len(), 1);
    assert!(
        sources[0].contains("while _budget_ok_() and (running):"),
        "the sandbox did not guard the loop: {}",
        sources[0]
    );
    drop(sources);
    let globals = host.globals_seen.lock().unwrap().clone().unwrap();
    assert!(globals.entries.contains_key("allow"));
    assert!(globals.entries.contains_key("_budget_ok_"));
}

#[test]
fn a_syntax_error_is_a_compilation_error_and_never_reaches_the_host() {
    let host = loaded_host();
    let Err(error) = CustomCodeGuardrail::new(
        params("def broken(:\n    pass\n"),
        as_host(&host),
        fixed_clock,
    ) else {
        panic!("expected a compilation error");
    };
    assert!(matches!(error, CustomCodeGuardrailError::Syntax { .. }));
    assert!(
        error
            .to_string()
            .starts_with("Syntax error in custom code:")
    );
    assert_eq!(host.load_count(), 0);
}

#[test]
fn a_forbidden_construct_is_a_compilation_error() {
    let host = loaded_host();
    let Err(error) = CustomCodeGuardrail::new(params("import os\n"), as_host(&host), fixed_clock)
    else {
        panic!("expected a compilation error");
    };
    assert!(matches!(error, CustomCodeGuardrailError::Syntax { .. }));
    assert!(error.to_string().contains("import"));
    assert_eq!(host.load_count(), 0);
}

#[test]
fn a_module_without_apply_guardrail_names_the_expected_signature() {
    let host = FakeHost::new(LoadReply::MissingFunction);
    let Err(error) = CustomCodeGuardrail::new(params(ALLOW_SOURCE), as_host(&host), fixed_clock)
    else {
        panic!("expected the missing function error");
    };
    assert_eq!(
        error.to_string(),
        "Custom code must define an 'apply_guardrail' function. Expected signature: apply_guardrail(inputs, request_data, input_type)"
    );
    assert!(matches!(error, CustomCodeGuardrailError::MissingFunction));
}

#[test]
fn a_non_callable_apply_guardrail_is_a_compilation_error() {
    let host = FakeHost::new(LoadReply::NotCallable);
    let Err(error) = CustomCodeGuardrail::new(params(ALLOW_SOURCE), as_host(&host), fixed_clock)
    else {
        panic!("expected the not callable error");
    };
    assert_eq!(
        error.to_string(),
        "'apply_guardrail' must be a callable function"
    );
}

#[test]
fn a_module_that_fails_to_load_is_a_compilation_error_with_the_cause() {
    let host = FakeHost::new(LoadReply::Failure(
        "NameError: name 'regex_match' is not defined",
    ));
    let Err(error) = CustomCodeGuardrail::new(params(ALLOW_SOURCE), as_host(&host), fixed_clock)
    else {
        panic!("expected a compilation error");
    };
    assert_eq!(
        error.to_string(),
        "Failed to compile custom code: NameError: name 'regex_match' is not defined"
    );
    assert!(matches!(
        error,
        CustomCodeGuardrailError::CompileFailed { .. }
    ));
}

#[test]
fn a_module_that_runs_past_the_timeout_is_a_compilation_error() {
    let host = FakeHost::new(LoadReply::Stuck);
    let Err(error) = CustomCodeGuardrail::new(
        params_with_timeout(ALLOW_SOURCE, 0.03),
        as_host(&host),
        fixed_clock,
    ) else {
        panic!("expected a compilation error");
    };
    assert!(
        error
            .to_string()
            .starts_with("Failed to compile custom code:")
    );
    assert!(error.to_string().contains("execution timeout"));
}
