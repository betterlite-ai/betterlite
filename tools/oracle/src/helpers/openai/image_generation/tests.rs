#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;

use super::Input;

const PREFIX: &str = "openai_image_generation-";

#[test]
fn helper_seeds_select_openai_image_generation() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("openai_image_generation"))
        })
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a seed has a UTF-8 name")
                .to_owned();
            let source = fs::read_to_string(&path).expect("a seed is readable");
            (name, source)
        })
        .collect();
    seeds.sort();
    assert!(!seeds.is_empty(), "no openai_image_generation seeds");
    assert!(seeds.iter().all(|(name, source)| {
        let case = serde_saphyr::from_str::<HelperCase>(source).expect("seed should parse");
        validate_helper(Path::new("<helper>"), &case).expect("seed should validate");
        let input = serde_json::from_value::<Input>(case.call.input.clone())
            .expect("seed should parse as the helper input");
        let expected = name
            .trim_start_matches(PREFIX)
            .trim_end_matches(".yaml")
            .replace('-', "_");
        case.call.operation == "openai_image_generation"
            && input.function.name() == expected
            && !input.probes.is_empty()
    }));
}
