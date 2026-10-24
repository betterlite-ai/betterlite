#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, parse_helper_case};
use crate::helpers::HelperCall;

#[test]
fn helper_seeds_select_catalog_generalizations() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_str().unwrap().contains("/catalog_generalizations"))
        .map(|path| {
            let source = fs::read_to_string(path).unwrap();
            parse_helper_case(&source).unwrap();
            serde_saphyr::from_str(&source).unwrap()
        })
        .collect();
    assert!(!seeds.is_empty());
    let selected = |seed: &HelperCase| matches!(seed.call, HelperCall::CatalogGeneralizations(_));
    assert!(seeds.iter().all(selected));
}
