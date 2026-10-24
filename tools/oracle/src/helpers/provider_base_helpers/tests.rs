#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, parse_helper_case};
use crate::helpers::HelperCall;

#[test]
fn helper_seeds_select_provider_base_helpers() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let file_name = path.file_name().and_then(|name| name.to_str());
            file_name.is_some_and(|name| name.starts_with("provider_base_helpers"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        parse_helper_case(source).is_ok() && matches!(case.call, HelperCall::ProviderBaseHelpers(_))
    }));
}
