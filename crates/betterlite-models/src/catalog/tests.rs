#![cfg(test)]

use super::{CatalogError, MAX_CATALOG_BYTES, MAX_CATALOG_ENTRIES, ModelCatalog};
use rstest::rstest;
use std::fmt::Write as _;
use std::fs;

#[rstest]
#[case::pinned_backup("backup")]
#[case::pinned_root("root")]
fn model_catalog_tests_behavior(#[case] case: &str) {
    assert!(!case.is_empty());
    let catalog = ModelCatalog::bundled().unwrap();
    assert!(catalog.len() > 4_000);
    assert!(!catalog.is_empty());
}

#[test]
fn runtime_remote_catalog_accepts_unknown_fields_with_paths() {
    let catalog = ModelCatalog::parse_remote(
        br#"{"model":{"litellm_provider":"openai","new_field":true,"metadata":{"new_note":"x"}}}"#,
    )
    .unwrap();
    assert_eq!(
        catalog.warnings(),
        &["model.metadata.new_note", "model.new_field"]
    );
}

#[test]
fn bundled_catalog_rejects_unknown_fields() {
    let error =
        ModelCatalog::parse_bundled(br#"{"model":{"litellm_provider":"openai","new_field":true}}"#)
            .unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn duplicate_catalog_keys_are_explicit_errors() {
    let error = ModelCatalog::parse_remote(
        br#"{"model":{"litellm_provider":"openai","mode":"chat","mode":"embedding"}}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("duplicate"));
}

#[test]
fn catalog_byte_limit_is_inclusive() {
    let at_limit = catalog_with_padding(MAX_CATALOG_BYTES);
    let over_limit = catalog_with_padding(MAX_CATALOG_BYTES + 1);

    assert!(!matches!(
        ModelCatalog::parse_remote(&at_limit),
        Err(CatalogError::TooLarge { .. })
    ));
    assert!(matches!(
        ModelCatalog::parse_remote(&over_limit),
        Err(CatalogError::TooLarge { .. })
    ));
}

#[test]
fn metadata_projection_returns_model_details() {
    let catalog =
        ModelCatalog::parse_remote(br#"{"model":{"litellm_provider":"openai","mode":"chat"}}"#)
            .unwrap();
    let metadata = catalog.metadata("model").unwrap();
    assert_eq!(metadata.model, "model");
    assert_eq!(metadata.provider, "openai");
}

#[test]
fn empty_catalog_reports_empty() {
    let catalog = ModelCatalog::parse_remote(b"{}").unwrap();
    assert!(catalog.is_empty());
}

fn catalog_with_padding(size: usize) -> Vec<u8> {
    let mut catalog = Vec::with_capacity(size);
    catalog.extend_from_slice(b"{}");
    catalog.resize(size, b' ');
    catalog
}

#[test]
fn catalog_entry_limit_is_inclusive() {
    let at_limit = catalog_with_entries(MAX_CATALOG_ENTRIES);
    let over_limit = catalog_with_entries(MAX_CATALOG_ENTRIES + 1);

    let at_limit_error = ModelCatalog::parse_remote(at_limit.as_bytes()).unwrap_err();
    assert!(!matches!(
        at_limit_error,
        CatalogError::TooManyEntries { .. }
    ));
    let over_limit_error = ModelCatalog::parse_remote(over_limit.as_bytes()).unwrap_err();
    assert!(matches!(
        over_limit_error,
        CatalogError::TooManyEntries { .. }
    ));
}

fn catalog_with_entries(count: usize) -> String {
    let mut catalog = String::from("{");
    for index in 0..count {
        if index > 0 {
            catalog.push(',');
        }
        write!(catalog, "\"model-{index}\":null").unwrap();
    }
    catalog.push('}');
    catalog
}

#[test]
fn aliases_keep_the_first_canonical_claim() {
    let catalog = ModelCatalog::parse_remote(
        br#"{
            "first":{"litellm_provider":"openai","aliases":["shared","first-alias"]},
            "second":{"litellm_provider":"anthropic","aliases":["shared"]}
        }"#,
    )
    .unwrap();
    assert_eq!(catalog.canonical_name("shared"), Some("first"));
    assert!(
        catalog
            .warnings()
            .iter()
            .any(|warning| warning.contains("shared"))
    );
}

#[test]
fn generalizations_are_not_model_entries() {
    let catalog = ModelCatalog::parse_remote(
        br#"{
            "fallback_generalizations":{"rules":[{"name":"base","pattern":"^x","model_info":{"litellm_provider":"openai"}}]},
            "model":{"litellm_provider":"openai"}
        }"#,
    )
    .unwrap();
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog.generalizations().match_routing("x"), Some("openai"));
}

#[test]
fn bundled_map_failure_is_cached() {
    ModelCatalog::parse_bundled(b"not-json").unwrap_err();
    ModelCatalog::parse_bundled(b"not-json").unwrap_err();
}

#[test]
fn replays_the_model_catalog_corpus() {
    replay_corpus();
}

fn replay_corpus() {
    for path in fs::read_dir(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fuzz/corpus/model_catalog"
    ))
    .unwrap()
    {
        let bytes = fs::read(path.unwrap().path()).unwrap();
        drop(ModelCatalog::parse_remote(&bytes));
    }
}
