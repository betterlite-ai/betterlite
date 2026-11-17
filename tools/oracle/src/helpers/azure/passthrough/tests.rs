#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Op;

const VARIANTS: [&str; 8] = [
    "body_model_inference_endpoint",
    "router_model_in_path",
    "foreign_deployment",
    "api_version_removal",
    "responses_stream_log",
    "messages_relay_log",
    "nonstream_usage",
    "passthrough_auth_inputs",
];

#[test]
fn helper_seeds_select_azure_passthrough() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap()
                .to_owned();
            (name, fs::read_to_string(path).unwrap())
        })
        .filter(|(name, _source)| name.starts_with("azure_passthrough"))
        .collect();
    assert!(!seeds.is_empty(), "no azure_passthrough seeds");
    let variants: BTreeSet<String> = seeds
        .iter()
        .map(|(name, source)| {
            let case: HelperCase = serde_saphyr::from_str(source).unwrap();
            assert_eq!(case.call.operation, "azure_passthrough");
            let variant = case.call.input["function"].as_str().unwrap().to_owned();
            assert!(
                VARIANTS.contains(&variant.as_str()),
                "{name} names {variant}"
            );
            Op::check(&case.call.input).unwrap();
            validate_helper(Path::new("<helper>"), &case).unwrap();
            variant
        })
        .collect();
    assert_eq!(variants.len(), VARIANTS.len());
}
