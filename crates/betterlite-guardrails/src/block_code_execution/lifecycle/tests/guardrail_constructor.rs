use betterlite_config::sections::content_filter_records::BlockCodeAction;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::block_code_execution::lifecycle::{
    BlockCodeExecutionError, BlockCodeExecutionGuardrail, DEFAULT_EVENT_HOOKS,
    DEFAULT_GUARDRAIL_NAME, EventHookSelection, SUPPORTED_EVENT_HOOKS,
};
use betterlite_guardrails::guardrail_base::error::GuardrailBaseError;
use rstest::rstest;

use crate::block_code_execution::support::{constructor_error, guardrail, params};

#[test]
fn defaults_match_the_documented_guardrail() {
    let built = guardrail(params());
    let settings = &built.base.settings;
    assert_eq!(
        settings.guardrail_name.as_deref(),
        Some(DEFAULT_GUARDRAIL_NAME)
    );
    assert_eq!(
        settings.supported_event_hooks.as_deref(),
        Some(SUPPORTED_EVENT_HOOKS.as_slice())
    );
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::Many(vec![
            "pre_call".to_owned(),
            "post_call".to_owned()
        ]))
    );
    assert!(!settings.default_on);
    assert!(built.blocked_languages.is_none());
    assert!(built.block_all);
    assert_eq!(built.scanner.action, BlockCodeAction::Block);
    assert_eq!(
        built.scanner.detector.confidence_threshold.to_bits(),
        0.5_f64.to_bits()
    );
    assert!(built.scanner.detect_execution_intent);
    assert!(built.scanner.detector.blocked_languages.is_empty());
}

#[rstest]
#[case::none(None, DEFAULT_GUARDRAIL_NAME)]
#[case::empty(Some(String::new()), DEFAULT_GUARDRAIL_NAME)]
#[case::named(Some("mine".to_owned()), "mine")]
fn the_guardrail_name_overrides_the_default(#[case] given: Option<String>, #[case] expected: &str) {
    let mut configured = params();
    configured.guardrail_name = given;
    let built = guardrail(configured);
    assert_eq!(
        built.base.settings.guardrail_name.as_deref(),
        Some(expected)
    );
}

#[rstest]
#[case::single(
    EventHookSelection::One("during_call".to_owned()),
    LitellmParamsMode::One("during_call".to_owned())
)]
#[case::list(
    EventHookSelection::Many(vec!["pre_call".to_owned(), "during_call".to_owned()]),
    LitellmParamsMode::Many(vec!["pre_call".to_owned(), "during_call".to_owned()])
)]
#[case::empty_list(
    EventHookSelection::Many(Vec::new()),
    LitellmParamsMode::Many(Vec::new())
)]
fn event_hook_normalizes_to_the_parent_mode(
    #[case] given: EventHookSelection,
    #[case] expected: LitellmParamsMode,
) {
    let mut configured = params();
    configured.event_hook = Some(given);
    let built = guardrail(configured);
    assert_eq!(built.base.settings.event_hook, Some(expected));
}

#[rstest]
#[case::strict(true)]
#[case::lenient(false)]
fn an_unknown_hook_name_is_rejected_even_when_modes_are_not_strict(#[case] strict: bool) {
    let mut configured = params();
    configured.event_hook = Some(EventHookSelection::One("nope".to_owned()));
    let error = constructor_error(BlockCodeExecutionGuardrail::new(configured, strict));
    assert!(matches!(
        error,
        BlockCodeExecutionError::Base(GuardrailBaseError::UnknownEventHook { hook }) if hook == "nope"
    ));
}

#[rstest]
#[case::strict(true, true)]
#[case::lenient(false, false)]
fn an_unsupported_but_valid_hook_follows_the_strict_modes_flag(
    #[case] strict: bool,
    #[case] rejected: bool,
) {
    let mut configured = params();
    configured.event_hook = Some(EventHookSelection::One("pre_mcp_call".to_owned()));
    let result = BlockCodeExecutionGuardrail::new(configured, strict);
    if rejected {
        assert!(matches!(
            &result,
            Err(BlockCodeExecutionError::Base(
                GuardrailBaseError::UnsupportedEventHook { hook, .. }
            )) if hook == "pre_mcp_call"
        ));
    } else {
        result.expect("a lenient guardrail accepts the hook");
    }
}

#[rstest]
#[case::below(-2.0_f64, 0.0_f64)]
#[case::just_below(0.499_9_f64, 0.499_9_f64)]
#[case::at_zero(0.0_f64, 0.0_f64)]
#[case::at_one(1.0_f64, 1.0_f64)]
#[case::above(3.0_f64, 1.0_f64)]
fn the_confidence_threshold_is_clamped(#[case] given: f64, #[case] expected: f64) {
    let mut configured = params();
    configured.confidence_threshold = given;
    let built = guardrail(configured);
    assert_eq!(
        built.scanner.detector.confidence_threshold.to_bits(),
        expected.to_bits()
    );
}

#[rstest]
#[case::absent(None, true)]
#[case::empty(Some(Vec::new()), true)]
#[case::listed(Some(vec!["python".to_owned()]), false)]
fn block_all_follows_the_blocked_language_list(
    #[case] given: Option<Vec<String>>,
    #[case] expected: bool,
) {
    let mut configured = params();
    configured.blocked_languages = given.clone();
    let built = guardrail(configured);
    assert_eq!(built.block_all, expected);
    assert_eq!(built.blocked_languages, given);
}

#[test]
fn the_action_and_intent_flag_reach_the_scanner() {
    let mut configured = params();
    configured.action = BlockCodeAction::Mask;
    configured.detect_execution_intent = false;
    let built = guardrail(configured);
    assert_eq!(built.scanner.action, BlockCodeAction::Mask);
    assert!(!built.scanner.detect_execution_intent);
}

#[test]
fn default_on_reaches_the_parent() {
    let mut configured = params();
    configured.default_on = true;
    let built = guardrail(configured);
    assert!(built.base.settings.default_on);
}

#[test]
fn the_default_hooks_match_the_wire_names() {
    assert_eq!(
        serde_json::to_string(&DEFAULT_EVENT_HOOKS).expect("serialize"),
        r#"["pre_call","post_call"]"#
    );
    let built = guardrail(params());
    assert_eq!(
        built.base.settings.event_hook,
        Some(LitellmParamsMode::Many(vec![
            "pre_call".to_owned(),
            "post_call".to_owned()
        ]))
    );
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::DuringCall
        ]
    );
}
