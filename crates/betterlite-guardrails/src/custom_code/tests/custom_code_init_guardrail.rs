use std::sync::Arc;

use super::fake::ALLOW_SOURCE;
use super::fake::FakeHost;
use super::fake::FakeRegistry;
use super::fake::LoadReply;
use super::fake::as_host;
use super::fake::fixed_clock;
use super::fake::object;
use super::fake::params;
use super::fake::params_with_timeout;
use super::fake::text;
use super::fake::value;
use crate::custom_code::initialize_guardrail;

fn loaded_host() -> Arc<FakeHost> {
    FakeHost::new(LoadReply::Module(value(object(vec![(
        "action",
        text("allow"),
    )]))))
}

#[test]
fn initialize_guardrail_builds_and_registers_the_guardrail() {
    let host = loaded_host();
    let mut registry = FakeRegistry::default();
    let guardrail = initialize_guardrail(
        params(ALLOW_SOURCE),
        as_host(&host),
        fixed_clock,
        &mut registry,
    )
    .unwrap();
    assert_eq!(guardrail.guardrail_name, "custom_code");
    assert_eq!(host.load_count(), 1);
    let callbacks = registry.callbacks.lock().unwrap();
    assert_eq!(callbacks.len(), 1);
    assert!(Arc::ptr_eq(&callbacks[0], &guardrail));
    drop(callbacks);
}

#[test]
fn initialize_guardrail_keeps_the_configured_timeout() {
    let host = loaded_host();
    let mut registry = FakeRegistry::default();
    let guardrail = initialize_guardrail(
        params_with_timeout(ALLOW_SOURCE, 4.0),
        as_host(&host),
        fixed_clock,
        &mut registry,
    )
    .unwrap();
    assert_eq!(guardrail.execution_timeout.to_bits(), 4.0_f64.to_bits());
}
