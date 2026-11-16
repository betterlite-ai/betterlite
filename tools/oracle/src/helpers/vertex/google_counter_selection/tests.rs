#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

fn run_case_file(file: &str) -> Value {
    use betterlite_oracle_helper_api::helper::Helper as _;

    let path = Path::new(ORACLE_DIR)
        .join("cases/helpers/google-counter-selection")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_google_counter_selection() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("google_counter_selection"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "google_counter_selection"
    }));
}

#[test]
fn oracle_counter_factory_kinds() {
    let result = run_case_file("counter-factory-kinds.yaml");
    assert_eq!(result["vertex"]["kind"], "VertexAITokenCounter");
    assert_eq!(result["studio"]["kind"], "GoogleAIStudioTokenCounter");
}

#[test]
fn oracle_beta_provider_is_not_selected() {
    let result = run_case_file("beta-not-selected.yaml");
    assert_eq!(result["vertex"]["selected"], json!([false, true]));
    assert_eq!(result["studio"]["selected"], json!([false, false]));
}

#[test]
fn oracle_each_provider_selects_its_own_counter() {
    let vertex = run_case_file("vertex-selected.yaml");
    assert_eq!(vertex["vertex"]["selected"], json!([true, false]));
    assert_eq!(vertex["studio"]["selected"], json!([false, false]));
    let studio = run_case_file("studio-selected.yaml");
    assert_eq!(studio["vertex"]["selected"], json!([false, false, false]));
    assert_eq!(studio["studio"]["selected"], json!([true, false, false]));
}
