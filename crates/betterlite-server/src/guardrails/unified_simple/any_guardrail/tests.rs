#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;

use super::AnyGuardrail;

struct SurfaceGuardrail;

impl GuardrailContract for SurfaceGuardrail {
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

#[test]
fn adapter_handle_forwards_the_whole_contract_surface() {
    let guardrail = AnyGuardrail(Arc::new(SurfaceGuardrail));
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
