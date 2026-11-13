use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::block_code_execution::lifecycle::{
    DEFAULT_EVENT_HOOKS, SUPPORTED_EVENT_HOOKS,
};

#[test]
fn supported_event_hooks_are_pre_call_post_call_and_during_call() {
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::DuringCall
        ]
    );
    assert_eq!(
        serde_json::to_string(&SUPPORTED_EVENT_HOOKS).expect("serialize"),
        r#"["pre_call","post_call","during_call"]"#
    );
}

#[test]
fn the_default_event_hooks_are_pre_call_and_post_call() {
    assert_eq!(
        DEFAULT_EVENT_HOOKS,
        [GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall]
    );
    assert_eq!(
        serde_json::to_string(&DEFAULT_EVENT_HOOKS).expect("serialize"),
        r#"["pre_call","post_call"]"#
    );
}
