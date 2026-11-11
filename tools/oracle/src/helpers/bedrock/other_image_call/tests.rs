#![cfg(test)]

use std::fs;
use std::path::Path;

use super::{Input, Op};
use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;

#[test]
fn helper_seeds_select_bedrock_other_image_call() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_other_image_call"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty(), "no bedrock_other_image_call seeds");
    assert!(seeds.iter().all(|source| {
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .is_ok()
            && serde_saphyr::from_str::<HelperCase>(source).is_ok_and(|case| {
                case.call.operation == "bedrock_other_image_call"
                    && serde_json::from_value::<Input>(case.call.input).is_ok()
            })
    }));
}

#[test]
fn a_provider_status_reaches_the_error_value() {
    let input: Input = serde_json::from_str(
        r#"{"function":"image_http_error","probes":[{"credentials":{"access_key_id":"AKIDEXAMPLE","secret_access_key":"secret"},"optional_params":{"aws_region_name":"us-east-1"},"reply":{"status":429,"body":"{\"message\":\"slow down\"}"}}]}"#,
    )
    .unwrap();
    let value = Op::run(&input).unwrap();
    assert_eq!(value[0]["status"], 429_i32);
    assert_eq!(value[0]["post_call"], false);
}
