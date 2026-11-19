#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

#[test]
fn helper_seeds_select_azure_files() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("the helper corpus folder exists")
        .filter_map(|entry| {
            let path = entry.expect("the corpus entry reads").path();
            let name = path.file_name()?.to_str()?.to_owned();
            name.starts_with("azure_files")
                .then(|| fs::read_to_string(path).expect("the seed reads"))
        })
        .collect();
    assert!(!seeds.is_empty(), "the azure files seeds exist");
    assert!(seeds.iter().all(|source| {
        let case = serde_saphyr::from_str::<HelperCase>(source).expect("the seed parses");
        case.call.operation == "azure_files"
            && validate_helper(Path::new("<helper>"), &case).is_ok()
    }));
}
