#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, parse_helper_case};
use crate::helpers::HelperCall;

#[test]
fn helper_seeds_select_provider_message_primitives() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_str().unwrap();
            name.starts_with("provider_message_primitives")
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    let selected = seeds.iter().all(|source| {
        parse_helper_case(source).unwrap();
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        matches!(case.call, HelperCall::ProviderMessagePrimitives(_))
    });
    assert!(selected);
}
