use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract as _;
use betterlite_guardrails::noma_v2::SUPPORTED_EVENT_HOOKS;
use betterlite_guardrails::noma_v2::error::NomaV2Error;
use betterlite_guardrails::noma_v2::initialize_guardrail_v2;
use betterlite_http::proxy::base_builder;
use tracing::Level;
use tracing::subscriber::with_default;

use crate::support::{CapturedLog, Registry, guardrail, lookup, params, settings};

#[test]
fn the_supported_event_hooks_are_the_five_noma_hooks() {
    let hooks = guardrail(settings("https://noma.example"))
        .get_supported_event_hooks()
        .unwrap();
    assert_eq!(hooks, SUPPORTED_EVENT_HOOKS.to_vec());
    assert_eq!(
        hooks,
        vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::DuringCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::PreMcpCall,
            GuardrailEventHooks::DuringMcpCall,
        ]
    );
}

#[test]
fn an_unsupported_hook_fails_strict_initialization() {
    let mut configured = params();
    configured.mode = Some(LitellmParamsMode::One(String::from("post_mcp_call")));
    let mut registry = Registry::default();
    let error = initialize_guardrail_v2(
        &configured,
        &lookup(&[("NOMA_API_KEY", "env-key")]),
        base_builder(),
        &mut registry,
    )
    .unwrap_err();
    assert!(matches!(error, NomaV2Error::Base { .. }));
    assert!(registry.callbacks.is_empty());
}

#[test]
fn strict_modes_off_keeps_an_unsupported_hook_with_a_warning() {
    let mut configured = params();
    configured.mode = Some(LitellmParamsMode::One(String::from("post_mcp_call")));
    configured.strict_modes = false;
    let mut registry = Registry::default();
    let log = CapturedLog::default();
    let env = lookup(&[("NOMA_API_KEY", "env-key")]);
    let outcome = with_default(log.subscriber(Level::WARN), || {
        initialize_guardrail_v2(&configured, &env, base_builder(), &mut registry)
    });
    assert!(outcome.is_ok());
    assert!(log.text().contains("LITELLM_STRICT_GUARDRAIL_MODES=false"));
}
