use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_judge::{GuardrailMode, TagValue};
use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::judge::hooks::{EventHookConfig, GuardrailEventHook, event_type_for};

#[derive(Debug, Clone, Copy)]
enum Hooks {
    PreCall,
    DuringCall,
    PostCall,
    LoggingOnly,
    PreAndPost,
    PreAndDuring,
    PostAndLogging,
    Tagged,
    EmptyMode,
}

fn config(hooks: Hooks) -> EventHookConfig {
    match hooks {
        Hooks::PreCall => EventHookConfig::Single(GuardrailEventHook::PreCall),
        Hooks::DuringCall => EventHookConfig::Single(GuardrailEventHook::DuringCall),
        Hooks::PostCall => EventHookConfig::Single(GuardrailEventHook::PostCall),
        Hooks::LoggingOnly => EventHookConfig::Single(GuardrailEventHook::LoggingOnly),
        Hooks::PreAndPost => EventHookConfig::Many(vec![
            GuardrailEventHook::PreCall,
            GuardrailEventHook::PostCall,
        ]),
        Hooks::PreAndDuring => EventHookConfig::Many(vec![
            GuardrailEventHook::PreCall,
            GuardrailEventHook::DuringCall,
        ]),
        Hooks::PostAndLogging => EventHookConfig::Many(vec![
            GuardrailEventHook::PostCall,
            GuardrailEventHook::LoggingOnly,
        ]),
        Hooks::Tagged => EventHookConfig::Mode(GuardrailMode {
            tags: BTreeMap::from([("judge".to_owned(), TagValue::One("pre_call".to_owned()))]),
            default: Some(TagValue::One("post_call".to_owned())),
        }),
        Hooks::EmptyMode => EventHookConfig::Mode(GuardrailMode {
            tags: BTreeMap::new(),
            default: None,
        }),
    }
}

#[rstest]
#[case::pre_call_request(
    Hooks::PreCall,
    GuardrailInputType::Request,
    Some(GuardrailEventHook::PreCall)
)]
#[case::during_call_request(
    Hooks::DuringCall,
    GuardrailInputType::Request,
    Some(GuardrailEventHook::DuringCall)
)]
#[case::logging_only_request(
    Hooks::LoggingOnly,
    GuardrailInputType::Request,
    Some(GuardrailEventHook::LoggingOnly)
)]
#[case::post_call_request(Hooks::PostCall, GuardrailInputType::Request, None)]
#[case::pre_and_post_request(
    Hooks::PreAndPost,
    GuardrailInputType::Request,
    Some(GuardrailEventHook::PreCall)
)]
#[case::pre_and_during_request(Hooks::PreAndDuring, GuardrailInputType::Request, None)]
#[case::tagged_request(
    Hooks::Tagged,
    GuardrailInputType::Request,
    Some(GuardrailEventHook::PreCall)
)]
#[case::empty_mode_request(Hooks::EmptyMode, GuardrailInputType::Request, None)]
#[case::post_call_response(
    Hooks::PostCall,
    GuardrailInputType::Response,
    Some(GuardrailEventHook::PostCall)
)]
#[case::logging_only_response(
    Hooks::LoggingOnly,
    GuardrailInputType::Response,
    Some(GuardrailEventHook::LoggingOnly)
)]
#[case::pre_call_response(Hooks::PreCall, GuardrailInputType::Response, None)]
#[case::pre_and_post_response(
    Hooks::PreAndPost,
    GuardrailInputType::Response,
    Some(GuardrailEventHook::PostCall)
)]
#[case::post_and_logging_response(Hooks::PostAndLogging, GuardrailInputType::Response, None)]
#[case::tagged_response(
    Hooks::Tagged,
    GuardrailInputType::Response,
    Some(GuardrailEventHook::PostCall)
)]
#[case::empty_mode_response(Hooks::EmptyMode, GuardrailInputType::Response, None)]
fn judge_event_type_tests_behavior(
    #[case] hooks: Hooks,
    #[case] input_type: GuardrailInputType,
    #[case] expected: Option<GuardrailEventHook>,
) {
    assert_eq!(event_type_for(&config(hooks), input_type), expected);
}
