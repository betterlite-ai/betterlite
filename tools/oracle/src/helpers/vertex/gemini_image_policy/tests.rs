#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::json;

use crate::case::HelperCase;
use crate::helpers::Helper as _;

use super::Op;
use super::ops::Input;

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut sources: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gemini_image_policy"))
        })
        .map(|path| fs::read_to_string(path).expect("seed should be readable"))
        .collect();
    sources.sort();
    sources
}

fn decode(source: &str) -> Input {
    let case: HelperCase = serde_saphyr::from_str(source).expect("seed should parse");
    assert_eq!(case.call.operation, "gemini_image_policy");
    serde_json::from_value(case.call.input).expect("seed input should decode")
}

#[test]
fn helper_seeds_select_gemini_image_policy() {
    let sources = seed_sources();
    assert!(!sources.is_empty());
    let inputs: Vec<Input> = sources.iter().map(|source| decode(source)).collect();
    assert!(inputs.iter().any(|input| !input.cases.is_empty()));
}

#[test]
fn an_invalid_image_config_string_is_an_unsupported_param() {
    let input: Input = serde_json::from_value(json!({
        "settings": {},
        "cases": [{
            "model": "gemini-2.5-flash-image",
            "catalog_supports_image_size": null,
            "params": {"imageConfig": "{bad"},
            "supported_params": ["n", "size", "imageConfig", "tools", "web_search_options"],
            "optional_params": {},
            "parse_image_config_string": true,
            "pre_mapped": null,
            "response": {}
        }]
    }))
    .expect("input should decode");
    let failure = Op::run(&input).expect_err("invalid JSON must fail");
    assert_eq!(failure.status, Some(400));
    assert_eq!(failure.class, "UnsupportedParamsError");
}
