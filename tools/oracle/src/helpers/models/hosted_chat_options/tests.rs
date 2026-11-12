#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

#[test]
fn helper_seeds_select_hosted_chat_options() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("hosted_chat_options"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .is_ok()
            && serde_saphyr::from_str::<HelperCase>(source)
                .unwrap()
                .call
                .operation
                == "hosted_chat_options"
    }));
    assert!(
        seeds
            .iter()
            .any(|source| source.contains("supports_reasoning: true")),
        "a seed must exercise the capability projection"
    );
    assert!(
        seeds
            .iter()
            .any(|source| { source.contains("tools:") && source.contains("drop_params: false") }),
        "a seed must exercise the Featherless tool rejection"
    );
}
