#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::body::{decode, decode_content_query};
use super::preview::Preview;
use super::workers::{WorkerBilling, WorkerName};

#[rstest]
fn replays_the_lens_admin_parsers_corpus(
    #[files("fuzz/corpus/lens_admin_parsers/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("a corpus seed is readable");
    if let Ok(query) = decode_content_query(&bytes) {
        assert!(query.offset >= 0);
    }
    drop(decode::<Preview>(&bytes));
    drop(decode::<WorkerBilling>(&bytes));
    drop(decode::<WorkerName>(&bytes));
}
