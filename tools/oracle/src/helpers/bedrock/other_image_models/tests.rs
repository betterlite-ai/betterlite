#![cfg(test)]
use std::fs;
use std::path::Path;

use super::{Input, Op};
use crate::case::{HelperCase, parse_helper_case};
use crate::helpers::Helper as _;

#[test]
fn provider_error_carries_its_status() {
    let input: Input = serde_json::from_str(
        r#"{"image_provider_error":{"family":"stability3","response_body":{"images":[],"finish_reasons":["ERROR","","CONTENT_FILTERED"]},"optional_params":{},"text":"a cat on a mat"}}"#,
    )
    .unwrap();
    let error = Op::run(&input).unwrap_err();
    assert_eq!(error.class, "BedrockError");
    assert_eq!(error.status, Some(400));
}

#[test]
fn helper_seeds_select_bedrock_other_image_models() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_other_image_models"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        parse_helper_case(source).is_ok()
            && serde_saphyr::from_str::<HelperCase>(source)
                .is_ok_and(|case| case.call.operation == "bedrock_other_image_models")
    }));
}
