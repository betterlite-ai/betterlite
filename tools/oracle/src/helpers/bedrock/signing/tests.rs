#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::{Helper as _, invoke};

use super::input::{Call, Input};

#[test]
fn helper_seeds_select_bedrock_signing() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .expect("the corpus directory should be readable")
        .map(|entry| entry.expect("a corpus entry should be readable").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_signing"))
        })
        .collect();
    assert!(!seeds.is_empty(), "the bedrock_signing seeds are missing");
    let variants: Vec<&'static str> = seeds
        .iter()
        .map(|path| {
            let source = fs::read_to_string(path).expect("a seed should be readable");
            let case: HelperCase = serde_saphyr::from_str(&source).expect("a seed should parse");
            validate_helper(path, &case).expect("a seed should validate");
            assert_eq!(case.call.operation, "bedrock_signing");
            super::Op::check(&case.call.input).unwrap();
            let input: Input =
                serde_json::from_value(case.call.input.clone()).expect("a seed should decode");
            assert!(!input.calls.is_empty());
            assert!(invoke(&case.call).ok, "{}", path.display());
            variant(&input.calls[0])
        })
        .collect();
    assert!(
        ["request_headers", "service_request", "off_loop"]
            .iter()
            .all(|expected| variants.contains(expected))
    );
}

fn variant(call: &Call) -> &'static str {
    match call {
        Call::RequestHeaders(_) => "request_headers",
        Call::ServiceRequest(_) => "service_request",
        Call::JsonPost(_) => "json_post",
        Call::RequestTarget(_) => "request_target",
        Call::BatchRequest(_) => "batch_request",
        Call::OffLoop(_) => "off_loop",
    }
}
