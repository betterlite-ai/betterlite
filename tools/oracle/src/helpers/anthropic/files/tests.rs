#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

#[test]
fn helper_seeds_select_anthropic_files() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| {
            let stem = path
                .file_stem()?
                .to_str()?
                .strip_prefix("anthropic_files-")?
                .to_owned();
            Some((stem, fs::read_to_string(path).unwrap()))
        })
        .collect();
    let selected: Vec<(String, String)> = seeds
        .iter()
        .map(|(stem, source)| {
            let case: HelperCase = serde_saphyr::from_str(source).unwrap();
            validate_helper(Path::new("<helper>"), &case).unwrap();
            assert_eq!(case.call.operation, "anthropic_files");
            let input = case.call.input.as_object().unwrap();
            assert_eq!(input.len(), 1);
            (stem.clone(), input.keys().next().unwrap().replace('_', "-"))
        })
        .collect();
    assert!(!selected.is_empty(), "no anthropic_files seeds");
    let (stems, variants): (Vec<String>, Vec<String>) = selected.into_iter().unzip();
    assert_eq!(stems, variants);
}
