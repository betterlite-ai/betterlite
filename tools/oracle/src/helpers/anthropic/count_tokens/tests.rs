#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

const VARIANTS: [&str; 6] = [
    "validate",
    "transform",
    "headers",
    "handle",
    "counter",
    "factory",
];

#[test]
fn helper_seeds_select_anthropic_count_tokens() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("anthropic_count_tokens"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .map(|source| serde_saphyr::from_str(&source).unwrap())
        .collect();
    assert!(!seeds.is_empty(), "no anthropic_count_tokens seeds");
    assert!(seeds.iter().all(|case| {
        let variant = case
            .call
            .input
            .as_object()
            .and_then(|fields| fields.keys().next());
        case.call.operation == "anthropic_count_tokens"
            && variant.is_some_and(|name| VARIANTS.contains(&name.as_str()))
            && validate_helper(Path::new("<helper>"), case).is_ok()
    }));
}
