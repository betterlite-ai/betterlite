#![cfg(test)]

use betterlite_config::sections::model_generalizations as config;
use betterlite_config::sections::settings::JsonValue;
use betterlite_router::generalization_integration::bounds::MAX_RULES_BYTES;
use betterlite_router::router_core::services::RouteServices as _;

use super::GeneralizationServices;

const ACME: &str = r#"[{"pattern": "^acme-", "model_info": {"litellm_provider": "acme"}}]"#;
const BETA: &str = r#"[{"pattern": "^beta-", "model_info": {"litellm_provider": "beta"}}]"#;

fn section(document: &str) -> config::Settings {
    let rules: Vec<JsonValue> = serde_json::from_str(document).unwrap();
    config::Settings { rules }
}

fn installed(document: &str) -> GeneralizationServices {
    let mut services = GeneralizationServices::default();
    services.reload(&section(document)).unwrap();
    services
}

#[test]
fn every_configured_rule_reaches_the_router_service() {
    let services = installed(
        r#"[{"pattern": "^acme-", "model_info": {"litellm_provider": "acme"}}, {"name": "broken", "pattern": "^broken"}]"#,
    );
    let service = services.model_generalizations().unwrap();
    assert_eq!(service.routing("acme-1").answer.as_deref(), Some("acme"));
    assert_eq!(service.routing("broken-1").answer, None);
    assert_eq!(service.engine_warnings.len(), 1);
}

#[test]
fn an_empty_section_installs_no_rules() {
    let services = installed("[]");
    let service = services.model_generalizations().unwrap();
    assert_eq!(service.routing("acme-1").answer, None);
    assert!(service.engine_warnings.is_empty());
}

#[test]
fn a_reload_replaces_the_installed_rules() {
    let mut services = installed(ACME);
    services.reload(&section(BETA)).unwrap();
    let service = services.model_generalizations().unwrap();
    assert_eq!(service.routing("beta-1").answer.as_deref(), Some("beta"));
    assert_eq!(service.routing("acme-1").answer, None);
}

#[test]
fn a_rule_document_over_the_byte_limit_fails_and_keeps_the_installed_rules() {
    let mut services = installed(ACME);
    let oversized = "a".repeat(MAX_RULES_BYTES);
    let document = format!(r#"[{{"pattern": "{oversized}", "model_info": {{}}}}]"#);
    assert!(services.reload(&section(&document)).is_err());
    let service = services.model_generalizations().unwrap();
    assert_eq!(service.routing("acme-1").answer.as_deref(), Some("acme"));
}
