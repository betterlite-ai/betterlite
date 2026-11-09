#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use crate::case::{HelperCase, helper::validate_helper};

fn corpus_paths() -> Vec<PathBuf> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}

fn parses(source: &str) -> bool {
    serde_saphyr::from_str(source)
        .map_err(|error| error.to_string())
        .and_then(|parsed| {
            validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
        })
        .is_ok()
}

#[test]
fn helper_seeds_select_anthropic_usage() {
    let seeds: Vec<String> = corpus_paths()
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("anthropic_usage"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        parses(source)
            && serde_saphyr::from_str::<HelperCase>(source)
                .unwrap()
                .call
                .operation
                == "anthropic_usage"
    }));
    assert!(
        corpus_paths()
            .iter()
            .all(|path| { parses(&fs::read_to_string(path).unwrap()) })
    );
}
