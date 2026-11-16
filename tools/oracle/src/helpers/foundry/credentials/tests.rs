#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

const VARIANTS: [&str; 8] = [
    "agent_headers",
    "agent_tokens",
    "auth_headers",
    "has_entra",
    "host_boundary",
    "native_responses",
    "reasoning_gate",
    "settings",
];

#[test]
fn helper_seeds_select_foundry_credentials() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let found: BTreeSet<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("foundry_credentials"))
        })
        .map(|path| seed_variant(&path))
        .collect();
    let expected: BTreeSet<String> = VARIANTS
        .iter()
        .map(|variant| (*variant).to_owned())
        .collect();
    assert_eq!(found, expected);
}

fn seed_variant(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let source = fs::read_to_string(path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    assert_eq!(case.call.operation, "foundry_credentials", "{name}");
    assert!(
        validate_helper(Path::new("<helper>"), &case).is_ok(),
        "{name}"
    );
    let input = case.call.input.as_object().unwrap();
    assert_eq!(input.len(), 1, "{name}");
    input.keys().next().unwrap().clone()
}
