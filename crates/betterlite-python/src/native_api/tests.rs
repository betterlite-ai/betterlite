#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::provider_model_sets::{ProviderModelSet, ProviderModelSets};

use super::{bundled_model_cost_json, core_version, provider_model_sets};

fn pinned_text() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../betterlite-models/fixtures/model_prices_and_context_window_backup.json");
    fs::read_to_string(path).unwrap()
}

#[test]
fn core_version_is_the_crate_version() {
    assert_eq!(core_version(), env!("CARGO_PKG_VERSION"));
    assert!(!core_version().is_empty());
}

#[test]
fn bundled_model_cost_json_is_the_pinned_text() {
    let text = bundled_model_cost_json().unwrap();
    assert_eq!(text, pinned_text());
    assert!(text.contains("\"sample_spec\""));
}

#[test]
fn provider_model_sets_has_one_sorted_list_per_set_name() {
    let sets = provider_model_sets();
    let names: BTreeSet<String> = ProviderModelSets::default()
        .sets
        .keys()
        .map(ToString::to_string)
        .collect();
    assert!(!names.is_empty());
    assert_eq!(sets.keys().cloned().collect::<BTreeSet<_>>(), names);
    assert!(sets.values().all(|list| list.is_sorted()));
}

#[test]
fn provider_model_sets_folds_the_bundled_catalog() {
    let catalog = ModelCatalog::parse(pinned_text().as_bytes(), UnknownFields::Reject).unwrap();
    let mut folded = ProviderModelSets::default();
    folded.add_known_models(&catalog);
    let want: BTreeMap<String, Vec<String>> = folded
        .sets
        .into_iter()
        .map(|(set, names)| (set.to_string(), names.into_iter().collect()))
        .collect();
    let sets = provider_model_sets();
    assert_eq!(sets, want);
    let chat = &sets[&ProviderModelSet::OpenAiChatCompletion.to_string()];
    assert!(chat.iter().any(|name| name == "gpt-4o"));
}
