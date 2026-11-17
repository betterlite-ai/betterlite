#![cfg(test)]

use betterlite_config::sections::model_generalizations as config;
use serde_json::json;

use super::service;

fn section() -> config::Settings {
    config::Settings {
        rules: vec![
            json!({"pattern": "^acme-", "model_info": {"litellm_provider": "acme"}}),
            json!({"name": "broken", "pattern": "^broken"}),
        ],
    }
}

#[test]
fn every_configured_rule_reaches_the_router_service() {
    let installed = service(&section()).unwrap();
    assert_eq!(installed.routing("acme-1").answer.as_deref(), Some("acme"));
    assert_eq!(installed.routing("broken-1").answer, None);
    assert_eq!(installed.engine_warnings.len(), 1);
}

#[test]
fn an_empty_section_installs_no_rules() {
    let installed = service(&config::Settings::default()).unwrap();
    assert_eq!(installed.routing("acme-1").answer, None);
    assert!(installed.engine_warnings.is_empty());
}
