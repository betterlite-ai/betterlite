#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;

#[test]
fn helper_seeds_select_openai_batches() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let file = path.file_name().and_then(|name| name.to_str());
            file.is_some_and(|name| name.starts_with("openai_batches"))
        })
        .map(|path| {
            let name = path.display().to_string();
            (name, fs::read_to_string(path).unwrap())
        })
        .collect();
    assert!(!seeds.is_empty(), "no openai_batches seeds");
    let selected = seeds.iter().all(|(name, source)| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        validate_helper(Path::new(name), &case).unwrap();
        case.call.operation == "openai_batches"
    });
    assert!(selected);
}
