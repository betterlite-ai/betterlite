#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use super::ProviderCache;
use crate::catalog::{ModelCatalog, UnknownFields};
use crate::json_provider_registry::JSONProviderRegistry;
use crate::provider_defaults_a::policies::ProviderPolicies;
use crate::provider_defaults_b::globals::GlobalCredentials;
use crate::resolution::ResolutionContext;

const CATALOG: &str = r#"{"gpt-4o": {"litellm_provider": "openai"}}"#;

fn context<'context>(
    catalog: &'context ModelCatalog,
    known: &'context BTreeSet<String>,
    environment: &'context BTreeMap<String, String>,
    custom: &'context BTreeSet<String>,
) -> ResolutionContext<'context> {
    ResolutionContext {
        catalog,
        known_model_names: known,
        custom_providers: custom,
        environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

#[test]
fn maps_a_catalog_name_to_its_provider_qualified_form() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();

    assert_eq!(
        cache.qualified("gpt-4o", &context),
        Some("openai/gpt-4o".to_owned())
    );
}

#[test]
fn normalizes_an_already_qualified_name_to_itself() {
    let catalog = ModelCatalog::default();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();

    assert_eq!(
        cache.qualified("openai/gpt-4o", &context),
        Some("openai/gpt-4o".to_owned())
    );
}

#[test]
fn returns_none_for_a_name_no_provider_claims() {
    let catalog = ModelCatalog::default();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();

    assert_eq!(cache.qualified("brand-new-unknown", &context), None);
    assert_eq!(cache.qualified("", &context), None);
}

#[test]
fn returns_none_when_the_resolved_model_name_is_empty() {
    let catalog = ModelCatalog::default();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();

    assert_eq!(cache.qualified("openai/", &context), None);
}

#[test]
fn caches_one_entry_per_model() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();

    let first = cache.qualified("gpt-4o", &context);
    let second = cache.qualified("gpt-4o", &context);
    assert_eq!(first, Some("openai/gpt-4o".to_owned()));
    assert_eq!(second, first);
    assert_eq!(cache.len(), 1);
}

#[test]
fn keeps_at_most_512_entries() {
    let catalog = ModelCatalog::default();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = context(&catalog, &known, &environment, &custom);
    let cache = ProviderCache::default();
    assert!(cache.is_empty());

    fill(&cache, &context, 513);

    assert!(!cache.is_empty());
    assert_eq!(cache.len(), 512);
    assert_eq!(
        cache.qualified("openai/model-0", &context),
        Some("openai/model-0".to_owned())
    );
}

fn fill(cache: &ProviderCache, context: &ResolutionContext<'_>, count: usize) {
    for index in 0..count {
        let model = format!("openai/model-{index}");
        assert_eq!(cache.qualified(&model, context), Some(model));
    }
}
