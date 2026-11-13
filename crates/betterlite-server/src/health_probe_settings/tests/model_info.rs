use betterlite_models::catalog::{ModelCatalog, UnknownFields};

use crate::health_probe_settings::model_info::{CatalogModelInfo, HealthCheckModelInfo};

const CATALOG: &[u8] = br#"{
  "chat-model": {"litellm_provider": "openai", "mode": "chat"},
  "embed-model": {"litellm_provider": "openai", "mode": "embedding"},
  "reasoning-model": {"litellm_provider": "openai", "mode": "chat", "supports_reasoning": true},
  "plain-model": {"litellm_provider": "openai", "mode": "chat", "supports_reasoning": false}
}"#;

fn catalog() -> ModelCatalog {
    ModelCatalog::parse(CATALOG, UnknownFields::Reject).unwrap()
}

#[test]
fn the_catalog_answers_a_mode() {
    let catalog = catalog();
    let info = CatalogModelInfo { catalog: &catalog };
    assert_eq!(info.mode("embed-model"), Some("embedding".to_owned()));
    assert_eq!(info.mode("chat-model"), Some("chat".to_owned()));
}

#[test]
fn an_unmapped_model_has_no_mode() {
    let catalog = catalog();
    let info = CatalogModelInfo { catalog: &catalog };
    assert_eq!(info.mode("mystery-model"), None);
}

#[test]
fn the_catalog_answers_reasoning_support() {
    let catalog = catalog();
    let info = CatalogModelInfo { catalog: &catalog };
    assert!(info.supports_reasoning("reasoning-model"));
    assert!(!info.supports_reasoning("plain-model"));
    assert!(!info.supports_reasoning("mystery-model"));
}
