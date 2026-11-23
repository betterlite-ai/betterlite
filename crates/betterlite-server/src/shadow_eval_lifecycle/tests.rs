#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::shadow_eval_lifecycle::body::decode_shadow_body;

mod shadow_eval_lifecycle_behaviors;
mod shadow_eval_lifecycle_boundaries;
mod support;

#[rstest]
fn replays_the_shadow_body_corpus(#[files("fuzz/corpus/shadow_body/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let parsed = decode_shadow_body(&bytes);
    let name = path.file_name().and_then(|name| name.to_str()).unwrap();
    if name.starts_with("shadow-eval-lifecycle-valid") {
        drop(parsed.unwrap());
    } else {
        drop(parsed.unwrap_err());
    }
}
