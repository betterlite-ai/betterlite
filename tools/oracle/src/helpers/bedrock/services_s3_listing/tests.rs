#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Op;

#[test]
fn helper_seeds_select_bedrock_services_s3_listing() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_services_s3_listing"))
        })
        .collect();
    assert!(
        !seeds.is_empty(),
        "no bedrock_services_s3_listing helper seeds"
    );
    let cases: Vec<HelperCase> = seeds
        .iter()
        .map(|path| {
            let case: HelperCase =
                serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
            validate_helper(path, &case).unwrap();
            Op::check(&case.call.input).unwrap();
            case
        })
        .collect();
    assert!(
        cases
            .iter()
            .all(|case| case.call.operation == "bedrock_services_s3_listing")
    );
}
