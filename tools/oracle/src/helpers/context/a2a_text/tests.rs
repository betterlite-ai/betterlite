#![cfg(test)]

use crate::case::{HelperCase, parse_helper_case};
use std::{fs, path::Path};

#[test]
fn helper_seeds_select_a2a_text() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut seeds = 0_usize;
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap().to_str().unwrap().starts_with("a2a_text") {
            let source = fs::read_to_string(path).unwrap();
            parse_helper_case(&source).unwrap();
            let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
            assert_eq!(case.call.operation, "a2a_text");
            seeds = seeds.saturating_add(1);
        }
    }
    assert!(seeds > 0_usize);
}
