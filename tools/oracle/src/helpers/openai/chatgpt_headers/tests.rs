#![cfg(test)]

use std::{fs, path::PathBuf};

use rstest::rstest;

use crate::case::HelperCase;
use crate::helpers::{HelperRun, run_case};

#[rstest]
fn matches_pinned_chatgpt_headers(#[files("cases/helpers/chatgpt-headers/*.yaml")] path: PathBuf) {
    let source = fs::read_to_string(path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    assert!(matches!(run_case(&case, false).unwrap(), HelperRun::Match));
}
