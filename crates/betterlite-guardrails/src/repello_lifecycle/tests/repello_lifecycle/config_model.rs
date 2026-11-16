use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract as _;
use betterlite_guardrails::repello_lifecycle::RepelloAILifecycle;

use crate::support::{guardrail, settings, text_field};

fn guarding() -> RepelloAILifecycle {
    guardrail(settings("http://127.0.0.1:1"))
}

#[test]
fn the_config_model_names_the_typed_dict_and_the_ui_label() {
    let model = guarding().get_config_model().unwrap();
    assert_eq!(
        text_field(&model, "model").as_deref(),
        Some("RepelloAIGuardrailConfigModel")
    );
    assert_eq!(
        text_field(&model, "ui_friendly_name").as_deref(),
        Some("RepelloAI Argus")
    );
}

#[test]
fn the_supported_event_hooks_are_pre_call_and_post_call() {
    assert_eq!(
        guarding().get_supported_event_hooks(),
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall
        ])
    );
}

#[test]
fn the_tool_result_and_full_request_defaults_are_untouched() {
    let guardrail = guarding();
    assert!(guardrail.supports_tool_result_scan());
    assert!(!guardrail.messages_cover_full_request());
}
