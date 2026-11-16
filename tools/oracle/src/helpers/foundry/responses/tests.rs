#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Op;

const VARIANTS: [(&str, &str); 5] = [
    ("foundry_responses-auth", "auth"),
    ("foundry_responses-decisions-route", "decisions_route"),
    ("foundry_responses-missing-base", "missing_base"),
    (
        "foundry_responses-no-native-websocket",
        "no_native_websocket",
    ),
    ("foundry_responses-url", "url"),
];

#[test]
fn helper_seeds_select_foundry_responses() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let paths: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("foundry_responses"))
        })
        .collect();
    assert!(!paths.is_empty(), "no foundry_responses helper case seeds");
    let found: BTreeMap<String, String> = paths
        .iter()
        .map(|path| (seed_stem(path), seed_variant(path)))
        .collect();
    let expected: BTreeMap<String, String> = VARIANTS
        .iter()
        .map(|(stem, variant)| ((*stem).to_owned(), (*variant).to_owned()))
        .collect();
    assert_eq!(found, expected);
}

fn seed_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_owned()
}

fn seed_variant(path: &Path) -> String {
    let name = seed_stem(path);
    let source = fs::read_to_string(path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    assert_eq!(case.call.operation, "foundry_responses", "{name}");
    assert!(
        validate_helper(Path::new("<helper>"), &case).is_ok(),
        "{name}"
    );
    assert!(Op::check(&case.call.input).is_ok(), "{name}");
    let steps: &Vec<serde_json::Value> = case
        .call
        .input
        .get("steps")
        .and_then(|steps| steps.as_array())
        .unwrap_or_else(|| panic!("{name} has no steps"));
    assert!(!steps.is_empty(), "{name}");
    let mut kinds = steps.iter().map(|step| {
        step.get("kind")
            .and_then(|kind| kind.as_str())
            .unwrap_or_default()
    });
    let first = kinds.next().unwrap_or_default().to_owned();
    assert!(kinds.all(|kind| kind == first), "{name}");
    first
}
