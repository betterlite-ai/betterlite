#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, parse_helper_case};

#[test]
fn helper_seeds_select_owned_parameter_names() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut seeds = Vec::new();
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("owned_parameter_names")
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .for_each(|source| seeds.push(source));
    assert!(!seeds.is_empty());
    assert!(seeds.into_iter().all(|source| {
        parse_helper_case(&source).unwrap();
        let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
        case.call.operation == "owned_parameter_names"
    }));
}
