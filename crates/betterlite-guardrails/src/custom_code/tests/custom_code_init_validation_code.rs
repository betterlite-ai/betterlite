use super::fake::FakeHost;
use super::fake::FakeRegistry;
use super::fake::LoadReply;
use super::fake::as_host;
use super::fake::fixed_clock;
use super::fake::object;
use super::fake::text;
use super::fake::value;
use crate::custom_code::config::CustomCodeGuardrailParams;
use crate::custom_code::initialize_guardrail;

#[test]
fn initialize_guardrail_requires_custom_code() {
    let host = FakeHost::new(LoadReply::Module(value(object(vec![(
        "action",
        text("allow"),
    )]))));
    let configured = CustomCodeGuardrailParams {
        custom_code: None,
        guardrail_name: Some("custom_code".to_owned()),
        timeout: None,
    };
    let mut registry = FakeRegistry::default();
    let Err(error) = initialize_guardrail(configured, as_host(&host), fixed_clock, &mut registry)
    else {
        panic!("expected the missing custom code error");
    };
    assert_eq!(
        error.to_string(),
        "Custom code guardrail requires 'custom_code' in litellm_params"
    );
    assert!(registry.callbacks.lock().unwrap().is_empty());
    assert_eq!(host.load_count(), 0);
}
