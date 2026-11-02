#![cfg(test)]

use betterlite_config::sections::cisco_verdict_contracts::{
    Configured, FallbackAction, FlaggedAction, InspectionType,
};
use secrecy::SecretString;

use crate::cisco_inspection::construct::CiscoInspectionGuardrail;

fn configured() -> Configured {
    Configured {
        guardrail_name: "cisco-ai-defense".to_owned(),
        api_key: SecretString::from("key".to_owned()),
        api_base: "https://cisco.test".to_owned(),
        inspection_type: InspectionType::Chat,
        inspect_path: "/api/v1/inspect/chat".to_owned(),
        enabled_rules: Vec::new(),
        on_flagged_action: FlaggedAction::Block,
        fallback_on_error: FallbackAction::Block,
        timeout: 10.0,
        integration_profile_id: None,
        integration_profile_version: None,
        integration_tenant_id: None,
        integration_type: None,
    }
}

#[test]
fn new_wraps_settings_and_client_without_change() {
    let settings = configured();
    let guardrail = CiscoInspectionGuardrail::new(settings.clone())
        .expect("default TLS backend builds a client");

    assert_eq!(guardrail.settings.guardrail_name, settings.guardrail_name);
    assert_eq!(guardrail.settings.api_base, settings.api_base);
    assert_eq!(guardrail.settings.inspection_type, InspectionType::Chat);
    assert_eq!(guardrail.settings.on_flagged_action, FlaggedAction::Block);
    assert_eq!(guardrail.settings.fallback_on_error, FallbackAction::Block);
}
