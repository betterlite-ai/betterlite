#![cfg(test)]

use std::{fs, path::Path};

use crate::case::{HelperCase, parse_helper_case};

fn seeds(prefix: &str) -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus).unwrap().filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(prefix))).map(|path| fs::read_to_string(path).unwrap()).collect()
}

fn assert_operation(prefix: &str) {
    let cases = seeds(prefix);
    assert!(!cases.is_empty());
    assert!(cases.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        parse_helper_case(source).is_ok() && case.call.operation == "otlp_normalization"
    }));
}

#[test]
fn otlp_truncate_helper() { assert_operation("otlp_normalization-truncate"); }

#[test]
fn otlp_elision_helper() { assert_operation("otlp_normalization-elision"); }

#[test]
fn otlp_content_text_helper() { assert_operation("otlp_normalization-content-text"); }

#[test]
fn otlp_ui_content_helper() { assert_operation("otlp_normalization-ui-content"); }

#[test]
fn otlp_response_encoding_helper() { assert_operation("otlp_normalization-response"); }
