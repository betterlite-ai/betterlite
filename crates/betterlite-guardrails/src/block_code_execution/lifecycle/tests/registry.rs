use betterlite_config::sections::guardrail_identity::enums::SupportedGuardrailIntegrations;
use betterlite_guardrails::block_code_execution::initializer::{
    GUARDRAIL_CLASS_REGISTRY, GUARDRAIL_INITIALIZER_REGISTRY, INTEGRATION, INTEGRATION_VALUE,
};

use crate::block_code_execution::support::{FakeRegistry, guardrail_entry, litellm_params, params};

#[test]
fn the_registry_key_is_the_supported_integration_value() {
    assert_eq!(
        INTEGRATION,
        SupportedGuardrailIntegrations::BlockCodeExecution
    );
    assert_eq!(
        serde_json::to_string(&INTEGRATION).expect("serialize"),
        "\"block_code_execution\""
    );
    assert_eq!(INTEGRATION_VALUE, "block_code_execution");
}

#[test]
fn the_initializer_registry_maps_the_key_to_initialize_guardrail() {
    assert_eq!(GUARDRAIL_INITIALIZER_REGISTRY.len(), 1);
    let (key, initializer) = GUARDRAIL_INITIALIZER_REGISTRY[0];
    assert_eq!(key, INTEGRATION_VALUE);
    let configured = litellm_params(r#"{"guardrail": "block_code_execution", "mode": "pre_call"}"#);
    let entry = guardrail_entry("registered", configured);
    let mut registry = FakeRegistry::default();
    let instance = initializer(&entry.litellm_params, &entry, true, &mut registry)
        .expect("the initializer builds the guardrail");
    assert_eq!(
        instance.base.settings.guardrail_name.as_deref(),
        Some("registered")
    );
    assert_eq!(registry.callbacks.len(), 1);
}

#[test]
fn the_class_registry_maps_the_key_to_the_guardrail_constructor() {
    assert_eq!(GUARDRAIL_CLASS_REGISTRY.len(), 1);
    let (key, class) = GUARDRAIL_CLASS_REGISTRY[0];
    assert_eq!(key, INTEGRATION_VALUE);
    let instance = class(params(), true).expect("the class builds the guardrail");
    assert_eq!(
        instance.base.settings.guardrail_name.as_deref(),
        Some("block_code_execution")
    );
}

#[test]
fn the_class_registry_accepts_a_configured_name() {
    let (_key, class) = GUARDRAIL_CLASS_REGISTRY[0];
    let mut configured = params();
    configured.guardrail_name = Some("custom".to_owned());
    let instance = class(configured, true).expect("the class builds the guardrail");
    assert_eq!(
        instance.base.settings.guardrail_name.as_deref(),
        Some("custom")
    );
}
