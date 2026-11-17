#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::too_many_lines,
        reason = "the seed test walks the shared corpus once and asserts each parsed case"
    )
)]

use std::collections::BTreeSet;
use std::fs;
use std::iter;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Op;

#[test]
fn helper_seeds_select_bedrock_other_titan_embeddings() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_other_titan_embeddings"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    let cases: Vec<HelperCase> = seeds
        .iter()
        .map(|source| {
            let case = serde_saphyr::from_str::<HelperCase>(source).unwrap();
            validate_helper(Path::new("<helper>"), &case).unwrap();
            assert_eq!(case.call.operation, "bedrock_other_titan_embeddings");
            case
        })
        .collect();
    let inputs: Vec<&serde_json::Value> = cases
        .iter()
        .flat_map(|case| iter::once(&case.call).chain(&case.calls))
        .map(|call| &call.input)
        .collect();
    let rejected: Vec<String> = inputs
        .iter()
        .copied()
        .filter_map(|input| <Op as Helper>::check(input).err())
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
            "titan_g1_request",
            "titan_v2_dimensions_normalize",
            "titan_v2_binary_embedding",
            "titan_multimodal_image",
            "cohere_v3_input_type",
            "token_count_response",
        ])
    );
}
