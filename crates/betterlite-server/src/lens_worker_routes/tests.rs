#![cfg(test)]

pub(crate) mod support;

use std::fs;
use std::path::PathBuf;

use axum::http::HeaderValue;
use rstest::rstest;

use crate::lens_worker_routes::{claim, content, credential};

#[rstest]
fn replays_the_lens_worker_parsers_corpus(
    #[files("fuzz/corpus/lens_worker_parsers/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("a corpus seed is readable");
    let authorization = HeaderValue::from_bytes(&bytes).ok();
    drop(claim::protocol_version(&bytes));
    drop(content::execution_id(&bytes));
    drop(credential::bearer_token(authorization.as_ref()));
}
