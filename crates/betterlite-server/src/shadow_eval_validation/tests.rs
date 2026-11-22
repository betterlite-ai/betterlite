#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::shadow_eval_validation::request::parse_shadow_eval_request;

mod shadow_eval_validation_behaviors;
mod shadow_eval_validation_boundaries;
mod support;

#[rstest]
fn replays_the_shadow_req_corpus(#[files("fuzz/corpus/shadow_req/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let parsed = parse_shadow_eval_request(&bytes);
    let name = path.file_name().and_then(|name| name.to_str()).unwrap();
    if name.starts_with("shadow-eval-validation-valid") {
        drop(parsed.unwrap());
    } else {
        drop(parsed.unwrap_err());
    }
}
