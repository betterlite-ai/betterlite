#![cfg(test)]

use crate::case::{HelperCase, parse_helper_case};
use std::fs;
use std::path::Path;

#[test]
fn helper_seeds_select_local_ttl_cache() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("local_ttl_cache"))
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
                == "local_ttl_cache"
    }));
}
