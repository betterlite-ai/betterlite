#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Op;

#[test]
fn helper_seeds_select_bedrock_services_s3_signing() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_services_s3_signing"))
        })
        .map(|path| fs::read_to_string(path).expect("seed should be readable"))
        .collect();
    assert!(!seeds.is_empty(), "no bedrock_services_s3_signing seeds");
    for source in &seeds {
        let parsed = serde_saphyr::from_str::<HelperCase>(source).expect("seed should parse");
        validate_helper(Path::new("<helper>"), &parsed).expect("seed should validate");
        assert_eq!(parsed.call.operation, "bedrock_services_s3_signing");
        Op::check(&parsed.call.input).expect("seed input should decode");
    }
}
