#![cfg(test)]

use super::{ConfigMap, IncludeError, LoadError, load_file, read_config, resolve_includes};
use crate::loader::ConfigLoader;
use crate::sections::Section as _;
use crate::sections::settings::settings_rules::JsonValue as Value;
use rstest::rstest;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/includes")
        .join(name)
}

fn loader() -> ConfigLoader<BTreeMap<String, String>> {
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
}

#[test]
fn converts_the_registered_section() {
    let settings = super::Raw { _marker: None }
        .convert(&BTreeMap::new())
        .unwrap();
    assert!(settings.converted);
}

#[test]
fn resolves_pinned_include_fixture() {
    let path = fixture("config_with_include.yaml");
    let config = resolve_includes(read_config(&path).unwrap(), &path).unwrap();

    assert_eq!(config["model_list"][0]["model_name"], "included-model");
    assert_eq!(
        config["litellm_settings"]["callbacks"],
        json!(["prometheus"])
    );
    assert!(!config.contains_key("include"));
}

#[test]
fn resolves_multiple_includes_in_order() {
    let path = fixture("config_with_multiple_includes.yaml");
    let config = resolve_includes(read_config(&path).unwrap(), &path).unwrap();
    let names: Vec<&str> = config["model_list"]
        .as_array()
        .unwrap()
        .iter()
        .map(|deployment| deployment["model_name"].as_str().unwrap())
        .collect();

    assert_eq!(names, ["included-model-1", "included-model-2"]);
}

#[test]
fn merge_extends_lists_and_replaces_other_values() {
    let base_settings: Value = serde_json::from_str(r#"{"base":true}"#).unwrap();
    let included_settings: Value = serde_json::from_str(r#"{"included":true}"#).unwrap();
    let base = ConfigMap::from_iter([
        (String::from("model_list"), json!(["base"])),
        (String::from("litellm_settings"), base_settings),
    ]);
    let included = ConfigMap::from_iter([
        (String::from("model_list"), json!(["included"])),
        (String::from("litellm_settings"), included_settings),
    ]);

    let merged = super::merge(base, included);

    assert_eq!(merged["model_list"], json!(["base", "included"]));
    let expected_settings: Value = serde_json::from_str(r#"{"included":true}"#).unwrap();
    assert_eq!(merged["litellm_settings"], expected_settings);
}

#[test]
fn resolves_includes_declared_by_included_files() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("root.yaml");
    let first = directory.path().join("first.yaml");
    let second = directory.path().join("second.yaml");
    fs::write(&root, "include: [first.yaml]\n").unwrap();
    fs::write(
        &first,
        "include: [second.yaml]\nmodel_list:\n  - model_name: first\n    litellm_params:\n      model: first\n",
    )
    .unwrap();
    fs::write(
        &second,
        "model_list:\n  - model_name: second\n    litellm_params:\n      model: second\n",
    )
    .unwrap();

    let config = load_file(&loader(), &root).unwrap();
    let names: Vec<&str> = config
        .model_list
        .iter()
        .map(|deployment| deployment.model_name.as_str())
        .collect();
    assert_eq!(names, ["first", "second"]);
}

#[test]
fn missing_include_names_the_file() {
    let error = load_file(&loader(), fixture("config_with_missing_include.yaml")).unwrap_err();

    let message = error.to_string();
    assert!(message.contains("non-existent-file.yaml"));
}

#[test]
fn fallback_uses_file_next_to_root() {
    let directory = tempdir().unwrap();
    let nested = directory.path().join("nested");
    fs::create_dir_all(&nested).unwrap();
    let root = directory.path().join("config.yaml");
    fs::write(&root, "include: [nested/child.yaml]\n").unwrap();
    fs::copy(
        fixture("config_include_fallback.yaml"),
        nested.join("child.yaml"),
    )
    .unwrap();
    fs::copy(
        fixture("fallback_models.yaml"),
        directory.path().join("fallback_models.yaml"),
    )
    .unwrap();
    let config = load_file(&loader(), &root).unwrap();

    assert_eq!(config.model_list[0].model_name, "fallback-model");
}

#[test]
fn declaring_relative_file_is_used_when_root_file_is_missing() {
    let directory = tempdir().unwrap();
    let nested = directory.path().join("nested");
    fs::create_dir_all(&nested).unwrap();
    let root = directory.path().join("config.yaml");
    let child = nested.join("child.yaml");
    fs::write(&root, "include: [nested/child.yaml]\n").unwrap();
    fs::write(&child, "include: [models.yaml]\n").unwrap();
    fs::write(
        nested.join("models.yaml"),
        "model_list:\n  - model_name: declaring-only-model\n    litellm_params:\n      model: gpt-4\n",
    )
    .unwrap();

    let config = load_file(&loader(), &root).unwrap();

    assert_eq!(config.model_list[0].model_name, "declaring-only-model");
}

#[test]
fn declaring_relative_file_wins_conflict() {
    let directory = tempdir().unwrap();
    let nested = directory.path().join("nested");
    fs::create_dir_all(&nested).unwrap();
    let root = directory.path().join("config.yaml");
    fs::write(&root, "include: [nested/child.yaml]\n").unwrap();
    fs::copy(
        fixture("config_include_conflict.yaml"),
        nested.join("child.yaml"),
    )
    .unwrap();
    fs::copy(
        fixture("conflict_models_root.yaml"),
        directory.path().join("conflict_models.yaml"),
    )
    .unwrap();
    fs::copy(
        fixture("conflict_models_declaring.yaml"),
        nested.join("conflict_models.yaml"),
    )
    .unwrap();
    let config = load_file(&loader(), &root).unwrap();

    assert_eq!(config.model_list[0].model_name, "declaring-model");
}

#[test]
fn cycles_skip_already_loaded_files() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("root.yaml");
    let child = directory.path().join("child.yaml");
    fs::write(&root, "include: [child.yaml]\nmodel_list: []\n").unwrap();
    fs::write(&child, "include: [root.yaml]\nmodel_list: []\n").unwrap();

    let config = load_file(&loader(), &root).unwrap();
    assert!(config.model_list.is_empty());
}

#[test]
fn cycles_skip_normalized_aliases_of_loaded_files() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("root.yaml");
    let child = directory.path().join("child.yaml");
    fs::write(&root, "include: [child.yaml]\nmodel_list: []\n").unwrap();
    fs::write(&child, "include: [./nested/../root.yaml]\nmodel_list: []\n").unwrap();

    let config = load_file(&loader(), &root).unwrap();
    assert!(config.model_list.is_empty());
}

#[test]
fn malformed_include_values_are_rejected() {
    let string_error =
        load_file(&loader(), fixture("config_include_malformed_string.yaml")).unwrap_err();
    assert!(string_error.to_string().contains("list of file paths"));

    let list_error =
        load_file(&loader(), fixture("config_include_malformed_list.yaml")).unwrap_err();
    assert!(list_error.to_string().contains("index 1"));

    let directory = tempdir().unwrap();
    let root = directory.path().join("root.yaml");
    let scalar = ConfigMap::from_iter([(String::from("include"), json!("child.yaml"))]);
    let error = resolve_includes(scalar, &root).unwrap_err();
    assert!(matches!(error, IncludeError::EntriesNotList));

    let entries = ConfigMap::from_iter([(String::from("include"), json!(["child.yaml", 4_i32]))]);
    let entry_error = resolve_includes(entries, &root).unwrap_err();
    assert!(matches!(
        entry_error,
        IncludeError::EntryNotString { index: 1 }
    ));
}

#[test]
fn load_file_reports_typed_loader_errors_as_a_source() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("root.yaml");
    fs::write(&root, "unknown_config_key: true\n").unwrap();

    let error = load_file(&loader(), root).unwrap_err();
    assert!(matches!(error, LoadError::Config { .. }));
    assert!(error.to_string().contains("unsupported config keys"));
}

fn write_include_corpus_candidates(directory: &Path, bytes: &[u8]) {
    for name in [
        "child.yaml",
        "included_models.yaml",
        "models_file_1.yaml",
        "models_file_2.yaml",
        "fallback_models.yaml",
        "conflict_models.yaml",
    ] {
        fs::write(directory.join(name), bytes).unwrap();
    }
}

#[rstest]
#[case::include("config_with_include.yaml")]
#[case::missing("config_with_missing_include.yaml")]
#[case::multiple("config_with_multiple_includes.yaml")]
#[case::fallback("config_include_fallback.yaml")]
#[case::conflict("config_include_conflict.yaml")]
#[case::malformed_string("config_include_malformed_string.yaml")]
#[case::malformed_list("config_include_malformed_list.yaml")]
#[case::cycle("config_include_cycle.yaml")]
#[case::included_models("included_models.yaml")]
#[case::models_one("models_file_1.yaml")]
#[case::models_two("models_file_2.yaml")]
#[case::fallback_models("fallback_models.yaml")]
#[case::conflict_models("conflict_models.yaml")]
fn replays_the_config_includes_corpus(#[case] corpus_name: &str) {
    let directory = tempdir().unwrap();
    let bytes = fs::read(Path::new("fuzz/corpus/config_includes").join(corpus_name)).unwrap();
    let root = directory.path().join("config.yaml");
    fs::write(&root, &bytes).unwrap();
    write_include_corpus_candidates(directory.path(), &bytes);
    drop(load_file(&loader(), root));
}
