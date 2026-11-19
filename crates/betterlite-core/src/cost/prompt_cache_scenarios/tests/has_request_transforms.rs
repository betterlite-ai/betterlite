#![cfg(test)]

use rstest::rstest;

use super::CallbackKind;
use super::LoggerTransforms;
use super::RequestHook;
use super::has_request_transforms;

fn custom(hooks: &[RequestHook]) -> LoggerTransforms<'_> {
    LoggerTransforms {
        kind: CallbackKind::Custom,
        overridden_hooks: hooks,
    }
}

fn builtin(hooks: &[RequestHook]) -> LoggerTransforms<'_> {
    LoggerTransforms {
        kind: CallbackKind::Builtin,
        overridden_hooks: hooks,
    }
}

#[rstest]
#[case::pre_call(&[RequestHook::PreCall])]
#[case::pre_request(&[RequestHook::PreRequest])]
#[case::pre_call_deployment(&[RequestHook::PreCallDeployment])]
#[case::several(&[RequestHook::PreCall, RequestHook::PreCallDeployment])]
fn has_request_transforms_is_true_for_an_overriding_custom_logger(#[case] hooks: &[RequestHook]) {
    assert!(has_request_transforms(&[custom(hooks)]));
}

#[test]
fn has_request_transforms_is_false_for_a_custom_logger_without_overrides() {
    assert!(!has_request_transforms(&[custom(&[])]));
}

#[rstest]
#[case::pre_call(&[RequestHook::PreCall])]
#[case::all_three(&[RequestHook::PreCall, RequestHook::PreRequest, RequestHook::PreCallDeployment])]
fn has_request_transforms_is_false_for_a_built_in_logger(#[case] hooks: &[RequestHook]) {
    assert!(!has_request_transforms(&[builtin(hooks)]));
}

#[test]
fn has_request_transforms_is_false_without_loggers() {
    assert!(!has_request_transforms(&[]));
}

#[test]
fn has_request_transforms_is_true_when_any_logger_transforms() {
    let loggers = [
        builtin(&[RequestHook::PreCall]),
        custom(&[RequestHook::PreRequest]),
    ];
    assert!(has_request_transforms(&loggers));
}

#[test]
fn has_request_transforms_is_false_when_no_logger_transforms() {
    let loggers = [builtin(&[RequestHook::PreCall]), custom(&[]), builtin(&[])];
    assert!(!has_request_transforms(&loggers));
}
