#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

#[test]
fn seeds_decode_the_gemini_request_input() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .expect("the helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gemini_request"))
        })
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a seed has a name")
                .to_owned();
            let source = fs::read_to_string(path).expect("a seed is readable");
            (name, source)
        })
        .collect();
    assert!(!seeds.is_empty(), "no gemini_request seed exists");
    for (name, source) in &seeds {
        let case: HelperCase =
            serde_saphyr::from_str(source).expect("a seed parses as a helper case");
        assert!(
            validate_helper(Path::new("<helper>"), &case).is_ok(),
            "{name} is a valid helper case"
        );
        assert_eq!(
            case.call.operation, "gemini_request",
            "{name} names the right operation"
        );
        let input: super::ops::Input =
            serde_json::from_value(case.call.input.clone()).expect("the seed input has the shape");
        assert!(!input.cases.is_empty(), "{name} carries at least one case");
    }
}
