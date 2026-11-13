#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;

use super::AnyGuardrail;

struct PreCallOnly;

impl GuardrailContract for PreCallOnly {
    fn get_config_model(&self) -> Option<GuardrailValue> {
        serde_json::from_str(r#"{"mode": "blocking"}"#).unwrap()
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall,
        ])
    }

    fn supports_tool_result_scan(&self) -> bool {
        false
    }

    fn messages_cover_full_request(&self) -> bool {
        true
    }
}

struct PostCallOnly;

impl GuardrailContract for PostCallOnly {
    fn get_config_model(&self) -> Option<GuardrailValue> {
        serde_json::from_str(r#"{"mode": "monitor"}"#).unwrap()
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        Some(vec![GuardrailEventHooks::DuringCall])
    }

    fn supports_tool_result_scan(&self) -> bool {
        true
    }

    fn messages_cover_full_request(&self) -> bool {
        false
    }
}

#[test]
fn adapter_handle_forwards_the_first_guardrail_surface() {
    let guardrail = AnyGuardrail(Arc::new(PreCallOnly));
    assert_eq!(
        guardrail.get_config_model(),
        serde_json::from_str(r#"{"mode": "blocking"}"#).unwrap()
    );
    assert_eq!(
        guardrail.get_supported_event_hooks(),
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall
        ])
    );
    assert!(!guardrail.supports_tool_result_scan());
    assert!(guardrail.messages_cover_full_request());
}

#[test]
fn adapter_handle_forwards_the_second_guardrail_surface() {
    let guardrail = AnyGuardrail(Arc::new(PostCallOnly));
    assert_eq!(
        guardrail.get_config_model(),
        serde_json::from_str(r#"{"mode": "monitor"}"#).unwrap()
    );
    assert_eq!(
        guardrail.get_supported_event_hooks(),
        Some(vec![GuardrailEventHooks::DuringCall])
    );
    assert!(guardrail.supports_tool_result_scan());
    assert!(!guardrail.messages_cover_full_request());
}
