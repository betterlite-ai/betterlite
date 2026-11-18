use betterlite_models::catalog::{ModelCatalog, UnknownFields};

use crate::model_deprecation::catalog::DeprecationCatalog;

use super::support::calendar_date;

const CATALOG: &str = r#"{
    "openai/dated": {"litellm_provider": "openai", "deprecation_date": "2026-05-10"},
    "openai/undated": {"litellm_provider": "openai"},
    "openai/no-provider": {"deprecation_date": "2026-07-01"},
    "openai/empty-provider": {"litellm_provider": "", "deprecation_date": "2026-08-01"},
    "aliased": {
        "litellm_provider": "alias-provider",
        "deprecation_date": "2026-06-01",
        "aliases": ["aliased-name"]
    }
}"#;

fn catalog() -> ModelCatalog {
    ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Warn).expect("the test catalog parses")
}

#[test]
fn catalog_lookup_resolves_keys_and_aliases() {
    let catalog = catalog();
    let dated = catalog
        .deprecation("openai/dated")
        .expect("the canonical key resolves");
    assert_eq!(dated.deprecation_date, calendar_date("2026-05-10"));
    assert_eq!(dated.litellm_provider.as_deref(), Some("openai"));

    let aliased = catalog
        .deprecation("aliased-name")
        .expect("the alias key resolves");
    assert_eq!(aliased.deprecation_date, calendar_date("2026-06-01"));
    assert_eq!(aliased.litellm_provider.as_deref(), Some("alias-provider"));
}

#[test]
fn catalog_lookup_drops_undated_and_default_providers() {
    let catalog = catalog();
    assert!(catalog.deprecation("openai/undated").is_none());
    assert!(catalog.deprecation("openai/missing").is_none());

    let absent = catalog
        .deprecation("openai/no-provider")
        .expect("the entry without a provider still resolves");
    assert_eq!(absent.litellm_provider, None);

    let empty = catalog
        .deprecation("openai/empty-provider")
        .expect("the empty provider entry still resolves");
    assert_eq!(empty.litellm_provider, None);
}
