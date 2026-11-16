#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};

use super::Input;

fn seeds() -> Vec<(String, HelperCase)> {
    let corpus =
        Path::new(betterlite_oracle_runner::helpers::ORACLE_DIR).join("fuzz/corpus/helper_case");
    let mut seeds: Vec<(String, HelperCase)> = fs::read_dir(corpus)
        .expect("the helper corpus")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("azure_images"))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).expect("a seed");
            let case: HelperCase = serde_saphyr::from_str(&source).expect("a helper case");
            (path.display().to_string(), case)
        })
        .collect();
    seeds.sort_by(|left, right| left.0.cmp(&right.0));
    seeds
}

#[test]
fn helper_seeds_select_azure_images() {
    let seeds = seeds();
    assert!(!seeds.is_empty());
    let rejected: Vec<&str> = seeds
        .iter()
        .filter(|(_, case)| {
            case.call.operation != "azure_images"
                || serde_json::from_value::<Input>(case.call.input.clone()).is_err()
                || validate_helper(Path::new("<helper>"), case).is_err()
        })
        .map(|(path, _)| path.as_str())
        .collect();
    assert!(rejected.is_empty(), "{rejected:?}");
}

#[test]
fn seeds_run_the_production_adapter() {
    for (path, case) in seeds() {
        let output = betterlite_oracle_runner::helpers::invoke(&[crate::GROUP], &case.call);
        assert!(output.ok, "{path} did not run");
    }
}
