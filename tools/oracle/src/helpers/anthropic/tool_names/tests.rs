#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, parse_helper_case};

#[test]
fn helper_seeds_select_anthropic_tool_names() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.starts_with("anthropic_tool_names"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        parse_helper_case(source).is_ok()
            && serde_saphyr::from_str::<HelperCase>(source)
                .unwrap()
                .call
                .operation
                == "anthropic_tool_names"
    }));
}
