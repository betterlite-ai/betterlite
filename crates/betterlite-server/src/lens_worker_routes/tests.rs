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
    if let Ok(version) = claim::protocol_version(&bytes) {
        // an accepted version is an integer, so its decimal text parses back to it
        assert_eq!(
            claim::protocol_version(version.to_string().as_bytes()),
            Ok(version)
        );
    }
    drop(content::execution_id(&bytes));
    drop(credential::bearer_token(authorization.as_ref()));
}
