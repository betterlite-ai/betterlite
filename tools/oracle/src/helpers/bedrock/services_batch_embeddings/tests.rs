#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Op;

const OPERATION: &str = "bedrock_services_batch_embeddings";

fn seed_sources() -> Vec<(String, String)> {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .expect("the corpus folder is readable")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(OPERATION))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).expect("a seed is readable");
            (path.display().to_string(), source)
        })
        .collect()
}

#[test]
fn helper_seeds_select_bedrock_services_batch_embeddings() {
    let seeds = seed_sources();
    assert!(!seeds.is_empty(), "no {OPERATION} seed exists");
    let cases: Vec<HelperCase> = seeds
        .iter()
        .map(|(path, source)| {
            let case: HelperCase = serde_saphyr::from_str(source).expect("a seed parses");
            validate_helper(Path::new(path), &case).expect("a seed validates");
            assert_eq!(case.call.operation, OPERATION);
            case
        })
        .collect();
    let inputs: Vec<&serde_json::Value> = cases
        .iter()
        .flat_map(|case| std::iter::once(&case.call).chain(&case.calls))
        .map(|call| &call.input)
        .collect();
    let rejected: Vec<String> = inputs
        .iter()
        .filter_map(|input| Op::check(input).err())
        .collect();
    assert!(rejected.is_empty(), "{rejected:#?}");
    // every branch of the helper has a seed, so a new branch cannot land without one
    let functions: BTreeSet<&str> = inputs
        .iter()
        .filter_map(|input| input.get("function").and_then(serde_json::Value::as_str))
        .collect();
    assert_eq!(
        functions,
        BTreeSet::from([
            "classify_records",
            "registry_schema",
            "name_boundary",
            "coerce_input",
            "map_embedding",
        ])
    );
}
