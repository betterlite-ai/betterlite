#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, parse_helper_case};

#[test]
fn helper_seeds_select_verbose_hooks() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("verbose_hooks"))
        })
        .map(|path| fs::read_to_string(path).expect("seed should be readable"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        parse_helper_case(source).is_ok()
            && serde_saphyr::from_str::<HelperCase>(source)
                .expect("seed should parse")
                .call
                .operation
                == "verbose_hooks"
    }));
}
