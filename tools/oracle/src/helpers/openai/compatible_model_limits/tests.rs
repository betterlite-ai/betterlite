#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};

fn run_case(file: &str) -> Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/compatible-model-limits")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_compatible_model_limits() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("compatible_model_limits"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "compatible_model_limits"
    }));
}

#[test]
fn limits_take_positive_integers_and_clamp_to_the_context() {
    let limits = run_case("positive-nonbool-and-clamp.yaml");
    assert_eq!(
        limits[0],
        json!({"max_tokens": 4096_i32, "max_input_tokens": 4096_i32, "max_output_tokens": 1024_i32})
    );
    assert_eq!(
        limits[1],
        json!({"max_tokens": 2048_i32, "max_input_tokens": 2048_i32, "max_output_tokens": 2048_i32})
    );
    assert_eq!(limits[2]["max_tokens"], 4096_i32);
    assert_eq!(limits[3]["max_tokens"], 512_i32);
    assert_eq!(limits[4], json!({"max_input_tokens": 7_i32}));
    assert_eq!(limits[5], json!({"max_input_tokens": 300_i32}));
}

#[test]
fn the_first_matching_id_wins_and_any_bad_list_is_empty() {
    let limits = run_case("first-id-and-empty-list.yaml");
    assert_eq!(limits[0]["max_tokens"], 10_i32);
    assert_eq!(limits.as_array().unwrap()[1..], vec![json!({}); 7]);
}
