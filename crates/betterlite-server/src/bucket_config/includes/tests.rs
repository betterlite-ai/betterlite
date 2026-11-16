#![cfg(test)]

use betterlite_config::sections::includes::ConfigMap;
use betterlite_config::sections::settings::settings_rules::JsonValue;
use rstest::rstest;

use super::{load_config_from_bucket, resolve_bucket_includes};
use crate::bucket_config::error::BucketConfigError;
use crate::bucket_config::source::{BucketConfigSource, BucketType};
use crate::bucket_config::test_support::MemoryReader;

fn reader(pairs: &[(&str, &str)]) -> MemoryReader {
    MemoryReader {
        objects: pairs
            .iter()
            .map(|(key, text)| ((*key).to_owned(), text.as_bytes().to_vec()))
            .collect(),
    }
}

fn config(yaml: &str) -> ConfigMap {
    serde_saphyr::from_str(yaml).unwrap()
}

fn value(json: &str) -> JsonValue {
    serde_json::from_str(json).unwrap()
}

fn source(object_key: &str) -> BucketConfigSource {
    BucketConfigSource {
        bucket_type: BucketType::S3,
        bucket_name: "bucket".to_owned(),
        object_key: object_key.to_owned(),
    }
}

const ROOT: &str = "config.yaml";

#[tokio::test]
async fn extends_lists_and_replaces_other_values() {
    let root = config(
        "model_list:\n  - model_name: root\nlitellm_settings:\n  drop_params: true\ninclude:\n  - child.yaml\n",
    );
    let reader = reader(&[(
        "child.yaml",
        "model_list:\n  - model_name: child\nlitellm_settings:\n  num_retries: 2\nrouter_settings:\n  routing_strategy: simple-shuffle\n",
    )]);

    let merged = resolve_bucket_includes(root, ROOT, &reader).await.unwrap();

    assert_eq!(
        merged.get("model_list"),
        Some(&value(
            r#"[{"model_name": "root"}, {"model_name": "child"}]"#
        ))
    );
    assert_eq!(
        merged.get("litellm_settings"),
        Some(&value(r#"{"num_retries": 2}"#))
    );
    assert_eq!(
        merged.get("router_settings"),
        Some(&value(r#"{"routing_strategy": "simple-shuffle"}"#))
    );
    assert!(!merged.contains_key("include"));
}

#[tokio::test]
async fn queues_nested_includes_after_the_existing_queue() {
    let root = config("include:\n  - a.yaml\n  - b.yaml\nsettings: root\n");
    let reader = reader(&[
        (
            "a.yaml",
            "include:\n  - c.yaml\nsettings: a\nfrom_a: true\n",
        ),
        ("b.yaml", "settings: b\n"),
        ("c.yaml", "settings: c\n"),
    ]);

    let merged = resolve_bucket_includes(root, ROOT, &reader).await.unwrap();

    assert_eq!(merged.get("settings"), Some(&value(r#""c""#)));
    assert_eq!(merged.get("from_a"), Some(&value("true")));
}

#[tokio::test]
async fn a_key_already_loaded_is_merged_once() {
    let root = config("model_list: []\ninclude:\n  - a.yaml\n  - a.yaml\n  - ./a.yaml\n");
    let reader = reader(&[("a.yaml", "model_list:\n  - model_name: child\n")]);

    let merged = resolve_bucket_includes(root, ROOT, &reader).await.unwrap();

    assert_eq!(
        merged.get("model_list"),
        Some(&value(r#"[{"model_name": "child"}]"#))
    );
}

#[tokio::test]
async fn the_root_key_counts_as_already_loaded() {
    let root = config("model_list: []\ninclude:\n  - config.yaml\n");
    let reader = reader(&[("config.yaml", "model_list:\n  - model_name: reread\n")]);

    let merged = resolve_bucket_includes(root, ROOT, &reader).await.unwrap();

    assert_eq!(merged.get("model_list"), Some(&value("[]")));
}

#[tokio::test]
async fn a_cycle_is_skipped() {
    let root = config("include:\n  - a.yaml\nroot: true\n");
    let reader = reader(&[
        ("a.yaml", "include:\n  - config.yaml\n  - b.yaml\na: true\n"),
        ("b.yaml", "include:\n  - a.yaml\nb: true\n"),
    ]);

    let merged = resolve_bucket_includes(root, ROOT, &reader).await.unwrap();

    assert_eq!(merged.get("root"), Some(&value("true")));
    assert_eq!(merged.get("a"), Some(&value("true")));
    assert_eq!(merged.get("b"), Some(&value("true")));
    assert!(!merged.contains_key("include"));
}

#[tokio::test]
async fn a_missing_included_object_is_the_exact_error() {
    let root = config("include:\n  - missing.yaml\n");
    let reader = reader(&[]);

    let error = resolve_bucket_includes(root, ROOT, &reader)
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Included config could not be read from bucket: missing.yaml. The underlying bucket error is logged above."
    );
    assert!(matches!(
        error,
        BucketConfigError::IncludedUnreadable { .. }
    ));
}

#[tokio::test]
async fn an_included_object_that_is_not_yaml_is_the_unreadable_error() {
    let root = config("include:\n  - bad.yaml\n");
    let reader = reader(&[("bad.yaml", "{unclosed: [")]);

    let error = resolve_bucket_includes(root, ROOT, &reader)
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Included config could not be read from bucket: bad.yaml. The underlying bucket error is logged above."
    );
}

#[tokio::test]
async fn an_included_object_that_is_not_a_mapping_is_the_mapping_error() {
    let root = config("include:\n  - list.yaml\n");
    let reader = reader(&[("list.yaml", "- one\n- two\n")]);

    let error = resolve_bucket_includes(root, ROOT, &reader)
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Config object in bucket is not a YAML mapping: list.yaml"
    );
    assert!(matches!(error, BucketConfigError::NotMapping { .. }));
}

#[tokio::test]
async fn an_empty_included_document_merges_nothing() {
    let root = config("model_list:\n  - model_name: root\ninclude:\n  - empty.yaml\n");
    let reader = reader(&[("empty.yaml", "")]);

    let merged = resolve_bucket_includes(root, ROOT, &reader).await.unwrap();

    assert_eq!(
        merged.get("model_list"),
        Some(&value(r#"[{"model_name": "root"}]"#))
    );
}

#[tokio::test]
async fn an_include_value_that_is_not_a_list_is_the_include_error() {
    let root = config("include: not-a-list\n");

    let error = resolve_bucket_includes(root, ROOT, &reader(&[]))
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "'include' must be a list of file paths");
    assert!(matches!(error, BucketConfigError::Include(_)));
}

#[tokio::test]
async fn an_include_entry_that_is_not_a_string_is_the_include_error() {
    let root = config("include:\n  - 1\n");

    let error = resolve_bucket_includes(root, ROOT, &reader(&[]))
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "'include' entry at index 0 must be a string path"
    );
}

#[tokio::test]
async fn load_config_from_bucket_returns_none_for_a_missing_root() {
    let loaded = load_config_from_bucket(&source(ROOT), &reader(&[]))
        .await
        .unwrap();
    assert!(loaded.is_none());
}

#[tokio::test]
async fn load_config_from_bucket_returns_none_for_an_empty_mapping() {
    let loaded = load_config_from_bucket(&source(ROOT), &reader(&[(ROOT, "{}")]))
        .await
        .unwrap();
    assert!(loaded.is_none());
}

#[tokio::test]
async fn load_config_from_bucket_returns_none_for_an_empty_document() {
    let loaded = load_config_from_bucket(&source(ROOT), &reader(&[(ROOT, "")]))
        .await
        .unwrap();
    assert!(loaded.is_none());
}

#[tokio::test]
async fn load_config_from_bucket_returns_none_for_an_invalid_root() {
    let loaded = load_config_from_bucket(&source(ROOT), &reader(&[(ROOT, "{unclosed: [")]))
        .await
        .unwrap();
    assert!(loaded.is_none());
}

#[rstest]
#[case::sequence("- one\n")]
#[case::scalar("just a string\n")]
#[tokio::test]
async fn load_config_from_bucket_rejects_a_non_mapping_root(#[case] text: &str) {
    let error = load_config_from_bucket(&source(ROOT), &reader(&[(ROOT, text)]))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Config object in bucket is not a YAML mapping: config.yaml"
    );
}

#[tokio::test]
async fn load_config_from_bucket_merges_a_root_with_includes() {
    let reader = reader(&[
        (
            ROOT,
            "include:\n  - team/models.yaml\nlitellm_settings:\n  drop_params: true\n",
        ),
        (
            "team/models.yaml",
            "model_list:\n  - model_name: included\n",
        ),
    ]);

    let loaded = load_config_from_bucket(&source(ROOT), &reader)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        loaded.get("model_list"),
        Some(&value(r#"[{"model_name": "included"}]"#))
    );
    assert_eq!(
        loaded.get("litellm_settings"),
        Some(&value(r#"{"drop_params": true}"#))
    );
    assert!(!loaded.contains_key("include"));
}
