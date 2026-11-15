use betterlite_models::catalog::{ModelCatalog, UnknownFields};

use crate::health_probe_settings::max_tokens::resolve_max_tokens;
use crate::health_probe_settings::model_info::{CatalogModelInfo, HealthCheckModelInfo as _};
use crate::health_probe_settings::tests::support::{health, object, policy};

const CATALOG: &[u8] = br#"{
  "chat-model": {"litellm_provider": "openai", "mode": "chat"},
  "embed-model": {"litellm_provider": "openai", "mode": "embedding"},
  "reasoning-model": {"litellm_provider": "openai", "mode": "chat", "supports_reasoning": true},
  "plain-model": {"litellm_provider": "openai", "mode": "chat", "supports_reasoning": false},
  "azure/gpt-5-chat": {"litellm_provider": "azure", "mode": "chat"},
  "gpt-5-chat": {"litellm_provider": "openai", "mode": "chat", "supports_reasoning": true},
  "azure/plain-chat": {"litellm_provider": "azure", "mode": "chat"},
  "plain-chat": {"litellm_provider": "openai", "mode": "chat", "supports_reasoning": false},
  "azure/fixed-chat": {"litellm_provider": "azure", "mode": "chat", "supports_reasoning": false},
  "fixed-chat": {"litellm_provider": "openai", "mode": "chat", "supports_reasoning": true}
}"#;

const NESTED_CATALOG: &[u8] = br#"{
  "vercel_ai_gateway/deepseek/deepseek-r1": {"litellm_provider": "vercel_ai_gateway", "mode": "chat"},
  "deepseek/deepseek-r1": {"litellm_provider": "deepseek", "mode": "chat", "supports_reasoning": true},
  "deepseek-r1": {"litellm_provider": "deepseek", "mode": "chat", "supports_reasoning": false},
  "azure/eu/o1-2024-12-17": {"litellm_provider": "azure", "mode": "chat"},
  "o1-2024-12-17": {"litellm_provider": "openai", "mode": "chat", "supports_reasoning": true},
  "novita/minimax/minimax-m2.1": {"litellm_provider": "novita", "mode": "chat"},
  "minimax/MiniMax-M2.1": {"litellm_provider": "minimax", "mode": "chat", "supports_reasoning": true},
  "minimax-m2.1": {"litellm_provider": "minimax", "mode": "chat", "supports_reasoning": false}
}"#;

const FIREWORKS_CATALOG: &[u8] = br#"{
  "fireworks_ai/deepseek-r1": {"litellm_provider": "fireworks_ai", "mode": "chat", "supports_reasoning": true}
}"#;

fn nested_catalog() -> ModelCatalog {
    ModelCatalog::parse(NESTED_CATALOG, UnknownFields::Reject).unwrap()
}

fn fireworks_catalog() -> ModelCatalog {
    ModelCatalog::parse(FIREWORKS_CATALOG, UnknownFields::Reject).unwrap()
}

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

#[test]
fn a_prefixed_entry_falls_back_to_the_bare_reasoning_flag() {
    let catalog = catalog();
    let info = CatalogModelInfo { catalog: &catalog };
    assert!(info.supports_reasoning("azure/gpt-5-chat"));
    assert!(!info.supports_reasoning("azure/plain-chat"));
}

#[test]
fn an_explicit_flag_on_the_prefixed_entry_wins_over_the_bare_one() {
    let catalog = catalog();
    let info = CatalogModelInfo { catalog: &catalog };
    assert!(!info.supports_reasoning("azure/fixed-chat"));
}

#[test]
fn a_prefixed_reasoning_route_takes_the_reasoning_limit() {
    let catalog = catalog();
    let models = CatalogModelInfo { catalog: &catalog };
    let tokens = resolve_max_tokens(
        &health(None, None, None, None),
        &object(r#"{"model":"azure/gpt-5-chat"}"#),
        &policy(None, Some(2048)),
        &models,
    );
    assert_eq!(tokens, Some(2048));
}

#[test]
fn a_prefixed_non_reasoning_route_takes_the_default_limit() {
    let catalog = catalog();
    let models = CatalogModelInfo { catalog: &catalog };
    let tokens = resolve_max_tokens(
        &health(None, None, None, None),
        &object(r#"{"model":"azure/plain-chat"}"#),
        &policy(None, Some(2048)),
        &models,
    );
    assert_eq!(tokens, Some(16));
}

#[test]
fn a_nested_route_reads_the_bare_entry_after_the_provider_prefix_only() {
    let catalog = nested_catalog();
    let info = CatalogModelInfo { catalog: &catalog };
    assert!(info.supports_reasoning("vercel_ai_gateway/deepseek/deepseek-r1"));
}

#[test]
fn a_nested_route_whose_remainder_is_unmapped_is_not_reasoning() {
    let catalog = nested_catalog();
    let info = CatalogModelInfo { catalog: &catalog };
    assert!(!info.supports_reasoning("azure/eu/o1-2024-12-17"));
}

#[test]
fn the_bare_entry_matches_without_case() {
    let catalog = nested_catalog();
    let info = CatalogModelInfo { catalog: &catalog };
    assert!(info.supports_reasoning("novita/minimax/minimax-m2.1"));
}

#[test]
fn an_unmapped_fireworks_fine_tune_reads_the_provider_index() {
    let catalog = fireworks_catalog();
    let models = CatalogModelInfo { catalog: &catalog };
    assert!(models.supports_reasoning("fireworks_ai/deepseek-r1-custom"));
    assert!(!models.supports_reasoning("openai/mystery"));
    let tokens = resolve_max_tokens(
        &health(None, None, None, None),
        &object(r#"{"model":"fireworks_ai/deepseek-r1-custom"}"#),
        &policy(None, Some(2048)),
        &models,
    );
    assert_eq!(tokens, Some(2048));
}
