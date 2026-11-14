#[cfg(test)]
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::test_map;

#[cfg(test)]
#[test]
fn resolution_returns_a_declared_provider_unchanged() {
    let kwargs = test_map([
        ("model", json!("some-model")),
        ("custom_llm_provider", json!("azure")),
    ]);
    assert_eq!(
        resolve_provider(&ModelCatalog::default(), &kwargs).as_deref(),
        Some("azure")
    );
}

#[cfg(test)]
#[test]
fn resolution_reads_the_provider_from_the_model_string() {
    let prefixed = test_map([
        ("model", json!("openai/gpt-4o")),
        ("custom_llm_provider", json!("")),
    ]);
    assert_eq!(
        resolve_provider(&ModelCatalog::default(), &prefixed).as_deref(),
        Some("openai")
    );
    let bare = test_map([
        ("model", json!("gpt-4o")),
        ("custom_llm_provider", json!("")),
    ]);
    let catalog = BundledCatalog::default().get();
    assert_eq!(resolve_provider(&catalog, &bare).as_deref(), Some("openai"));
}

#[cfg(test)]
#[test]
fn resolution_returns_none_for_an_unknown_model_without_a_provider() {
    let kwargs = test_map([("model", json!("zzz-unmapped-model"))]);
    let catalog = BundledCatalog::default().get();
    assert!(resolve_provider(&catalog, &kwargs).is_none());
}

#[cfg(test)]
#[test]
fn resolution_returns_none_for_a_model_that_is_not_a_string() {
    let kwargs = test_map([("model", json!(5)), ("custom_llm_provider", json!(""))]);
    let catalog = ModelCatalog::default();
    assert!(resolve_provider(&catalog, &kwargs).is_none());
    assert!(resolve_provider(&catalog, &Map::new()).is_none());
}
