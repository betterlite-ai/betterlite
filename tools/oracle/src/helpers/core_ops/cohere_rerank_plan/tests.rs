#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use betterlite_oracle::case::helper::{HelperCase, validate_helper};
use serde_json::Value;

#[derive(serde::Deserialize)]
struct SeedCall {
    call: BTreeMap<String, Value>,
}

fn seed_calls(name: &str, source: &str) -> bool {
    let seed: SeedCall = serde_saphyr::from_str(source).expect("the seed call parses");
    let Some(value) = seed.call.get("cohere_rerank_plan") else {
        return false;
    };
    let input: super::Input =
        serde_json::from_value(value.clone()).expect("the seed input has the typed shape");
    assert!(
        !input.calls.is_empty(),
        "{name} must carry at least one call"
    );
    true
}

#[test]
fn replays_the_cohere_rerank_plan_seeds() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .expect("the helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("cohere_rerank_plan"))
        })
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a seed has a name")
                .to_owned();
            let source = fs::read_to_string(path).expect("a seed is readable");
            (name, source)
        })
        .collect();
    assert!(!seeds.is_empty(), "no cohere_rerank_plan seed exists");
    assert!(seeds.iter().all(|(name, source)| {
        let case: HelperCase =
            serde_saphyr::from_str(source).expect("a seed parses as a helper case");
        validate_helper(Path::new("<helper>"), &case).is_ok() && seed_calls(name, source)
    }));
}
