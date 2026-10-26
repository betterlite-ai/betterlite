#![cfg(test)]

use super::super::enums::{PiiAction, SupportedGuardrailIntegrations};

#[test]
fn wire_values_round_trip() {
    let json = serde_json::to_string(&SupportedGuardrailIntegrations::GenericGuardrailApi).unwrap();
    assert_eq!(json, "\"generic_guardrail_api\"");
    let action: PiiAction = serde_json::from_str("\"MASK\"").unwrap();
    assert_eq!(action, PiiAction::Mask);
}
