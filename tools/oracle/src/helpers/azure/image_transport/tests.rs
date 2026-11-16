#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

#[test]
fn helper_seeds_select_azure_image_transport() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("azure_image_transport"))
        })
        .map(|path| {
            (
                path.display().to_string(),
                fs::read_to_string(path).unwrap(),
            )
        })
        .collect();
    assert!(!seeds.is_empty(), "no azure_image_transport seeds");
    let selected = seeds.iter().all(|(name, source)| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        validate_helper(Path::new(name), &case).unwrap();
        case.call.operation == "azure_image_transport"
    });
    assert!(selected);
}
